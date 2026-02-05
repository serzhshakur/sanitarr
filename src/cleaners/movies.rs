use crate::{
    cleaners::utils,
    config::RadarrConfig,
    http::{
        Item as JellyfinItem, ItemsFilter, JellyfinClient, Movie, MovieEditor, RadarrClient,
        TorrentClientKind, UserId,
    },
    services::DownloadService,
};
use log::{debug, info, warn};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

pub struct MoviesCleaner {
    radarr_client: RadarrClient,
    jellyfin: JellyfinClient,
    download_service: DownloadService,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
    user_id: UserId,
    unmonitor_watched: bool,
}

/// MoviesCleaner is responsible for cleaning up watched movies from Radarr and
/// Download client (e.g. qBittorrent).
impl MoviesCleaner {
    pub fn new(
        radarr_config: RadarrConfig,
        jellyfin: JellyfinClient,
        download_service: DownloadService,
        user_id: &UserId,
    ) -> anyhow::Result<Self> {
        let RadarrConfig {
            base_url,
            api_key,
            tags_to_keep,
            retention_period,
            unmonitor_watched,
        } = radarr_config;
        let radarr_client = RadarrClient::new(&base_url, &api_key)?;

        Ok(Self {
            radarr_client,
            jellyfin,
            download_service,
            tags_to_keep,
            retention_period,
            unmonitor_watched,
            user_id: user_id.clone(),
        })
    }

    /// unmonitor watched movies (if configured) and cleanup movies from Radarr
    /// and Download client that are fully watched in Jellyfin
    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let watched_movies = self.watched_movies(&self.user_id).await?;
        if watched_movies.is_empty() {
            log::info!("no movies found for deletion in Jellyfin!");
            return Ok(());
        }

        if self.unmonitor_watched {
            self.unmonitor(&watched_movies).await?;
        }

        let forbidden_tags = self.forbidden_tags().await?;
        let movies_for_deletion =
            watched_movies.filter_for_deletion(self.retention_period, &forbidden_tags)?;

        if movies_for_deletion.is_empty() {
            info!("no movies found for deletion in Radarr!");
            return Ok(());
        }

        let movie_ids = movies_for_deletion.iter().map(|m| m.id).collect();
        let download_ids = self.download_ids(&movie_ids).await?;

        if force_delete {
            debug!("trying to delete items in Radarr: {movies_for_deletion:?}");
            self.delete_movies(&movie_ids).await?;
            info!("successfully deleted items from Radarr: {movies_for_deletion:?}");
            self.download_service.delete(&download_ids).await?;
        } else {
            info!(
                "no items will be deleted as no `--force-delete` flag is provided. Listing them instead: {movies_for_deletion:?}"
            );
            self.download_service.list(&download_ids).await?;
        }

        Ok(())
    }

    /// queries Jellyfin and returns all watched movies for the given user
    async fn watched_jellyfin_items(&self, user_id: &UserId) -> anyhow::Result<Vec<JellyfinItem>> {
        self.jellyfin
            .items(
                ItemsFilter::watched()
                    .user_id(user_id.as_ref())
                    .include_item_types(&["Movie", "Video"]),
            )
            .await
    }

    /// delete movies with given ids
    async fn delete_movies(&self, movies_ids: &HashSet<u64>) -> anyhow::Result<()> {
        let delete_futs = movies_ids
            .iter()
            .map(|id| self.radarr_client.delete_movie(*id));
        let _ = futures::future::try_join_all(delete_futs).await?;
        Ok(())
    }

    /// unmonitor watched movies that are still monitored
    async fn unmonitor(&self, watched: &WatchedMovies) -> anyhow::Result<()> {
        let ids = watched.monitored_movie_ids();
        if ids.is_empty() {
            debug!("no monitored movies found for unmonitoring");
        } else {
            let request = MovieEditor::new(ids).monitored(false);
            let response = self.radarr_client.bulk_edit(&request).await?;
            let log_msg = response
                .iter()
                .map(|m| format!("  - {m}"))
                .collect::<Vec<_>>()
                .join("\n");
            info!("unmonitored movies in Radarr: \n{log_msg}");
        }
        Ok(())
    }

    /// queries Radarr history for given movie ids and gets corresponding
    /// download_id's per torrent client for each
    async fn download_ids(
        &self,
        ids: &HashSet<u64>,
    ) -> anyhow::Result<HashMap<TorrentClientKind, HashSet<String>>> {
        let mut per_client_hashes = HashMap::new();
        let records = self.radarr_client.history_records(ids).await?;
        for record in records {
            if let Some((kind, hash)) = record.download_id_per_client() {
                per_client_hashes
                    .entry(kind)
                    .or_insert_with(HashSet::new)
                    .insert(hash);
            }
        }
        Ok(per_client_hashes)
    }

    /// gets IDs of the tags that are configured to be kept
    async fn forbidden_tags(&self) -> anyhow::Result<Vec<u64>> {
        debug!("forbidden movie tags configured: {:?}", self.tags_to_keep);

        let tags = self.radarr_client.tags().await?;
        let forbidden_tags = tags
            .iter()
            .filter(|t| self.tags_to_keep.contains(&t.label))
            .map(|t| t.id)
            .collect();

        debug!("forbidden tag ids: {forbidden_tags:?}");

        Ok(forbidden_tags)
    }

    /// queries movies per Jellyfin items and returns a [`WatchedMovies`] object
    async fn watched_movies(&self, user_id: &UserId) -> anyhow::Result<WatchedMovies> {
        let items = self.watched_jellyfin_items(user_id).await?;
        let movies_futs = items.into_iter().map(|jellyfin_item| async move {
            let Some(tmdb_id) = jellyfin_item.tmdb_id() else {
                warn!("movie \"{}\" has no TMDB id, skipping", jellyfin_item.name);
                return Ok(None);
            };
            let movies: Vec<Movie> = self.radarr_client.movies_by_tmdb_id(tmdb_id).await?;
            let watched = WatchedMovie {
                jellyfin_item,
                movies,
            };
            Ok::<_, anyhow::Error>(Some(watched))
        });

        let results = futures::future::try_join_all(movies_futs).await?;
        let movie_items = results.into_iter().flatten().collect();
        Ok(WatchedMovies(movie_items))
    }
}

/// check if it's safe to delete a movie.
fn safe_to_delete(movie: &Movie, forbidden_tags: &[u64]) -> bool {
    let has_forbidden_tags = movie
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| forbidden_tags.contains(tag)));

    if has_forbidden_tags {
        debug!("movie '{}' has forbidden tags, skipping", movie.title);
        return false;
    }
    true
}

struct WatchedMovie {
    jellyfin_item: JellyfinItem,
    movies: Vec<Movie>,
}

struct WatchedMovies(Vec<WatchedMovie>);

impl WatchedMovies {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn movies(&self) -> Vec<&Movie> {
        self.0.iter().flat_map(|wm| wm.movies.iter()).collect()
    }

    fn monitored_movie_ids(&self) -> HashSet<u64> {
        self.0
            .iter()
            .flat_map(|wm| wm.movies.iter())
            .filter_map(|m| m.monitored.then_some(m.id))
            .collect()
    }

    fn filter_for_deletion(
        &self,
        retention_period: Option<Duration>,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<Vec<&Movie>> {
        let movies = match retention_period {
            Some(retention_period) => {
                let retention_date = chrono::Utc::now() - retention_period;
                let mut safe_to_delete_items = vec![];

                for item in &self.0 {
                    if let Some(last_played) = item.jellyfin_item.last_played_date() {
                        if retention_date > last_played {
                            safe_to_delete_items.extend(&item.movies);
                        } else {
                            debug!(
                                "retention period for \"{}\" is not yet passed ({} left), skipping",
                                item.jellyfin_item.name,
                                utils::retention_str(&last_played, &retention_date)
                            );
                        }
                    };
                }
                safe_to_delete_items
            }
            None => {
                if !self.0.is_empty() {
                    warn!(
                        "no retention period is set for Radarr, will delete all movies immediately"
                    );
                }
                self.movies()
            }
        };

        let movies = movies
            .into_iter()
            .filter(|movie| safe_to_delete(movie, forbidden_tags))
            .collect();

        Ok(movies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_movie_safe_to_delete() {
        let movie = Movie {
            id: 1,
            monitored: false,
            tags: Some(vec![1, 2, 3]),
            title: "movie".to_string(),
        };
        assert!(safe_to_delete(&movie, &[]));
    }

    #[test]
    fn test_movie_not_safe_to_delete_forbidden_tags() {
        let movie = Movie {
            id: 1,
            monitored: false,
            tags: Some(vec![5]),
            title: "movie".to_string(),
        };
        assert!(!safe_to_delete(&movie, &[4, 5, 6]));
    }
}
