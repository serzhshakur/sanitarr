use crate::{
    cleaners::utils,
    config::RadarrConfig,
    http::{Item, ItemsFilter, JellyfinClient, Movie, RadarrClient, TorrentClientKind, UserId},
    services::DownloadService,
};
use log::{debug, info, warn};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

pub struct MoviesCleaner {
    radarr_client: RadarrClient,
    jellyfin: Arc<JellyfinClient>,
    download_client: Arc<DownloadService>,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
}

/// MoviesCleaner is responsible for cleaning up watched movies from Radarr and
/// Download client (e.g. qBittorrent).
impl MoviesCleaner {
    pub fn new(
        radarr_config: RadarrConfig,
        jellyfin: Arc<JellyfinClient>,
        download_client: Arc<DownloadService>,
    ) -> anyhow::Result<Self> {
        let RadarrConfig {
            base_url,
            api_key,
            tags_to_keep,
            retention_period,
        } = radarr_config;
        let radarr_client = RadarrClient::new(&base_url, &api_key)?;

        Ok(Self {
            radarr_client,
            jellyfin,
            download_client,
            tags_to_keep,
            retention_period,
        })
    }

    pub async fn cleanup(&self, user_name: &str, force_delete: bool) -> anyhow::Result<()> {
        let user = self.jellyfin.user(user_name).await?;
        let items = self.watched_items(&user.id).await?;
        if items.is_empty() {
            log::info!("no movies found for deletion in Jellyfin!");
            return Ok(());
        }

        let movies = self.movies_for_deletion(&items).await?;

        if movies.is_empty() {
            info!("no movies found for deletion in Radarr!");
            return Ok(());
        }

        let movie_ids = movies.iter().map(|m| m.id).collect();
        let download_ids = self.download_ids(&movie_ids).await?;

        if force_delete {
            debug!("trying to delete items in Radarr: {movies:?}");
            self.delete_movies(&movie_ids).await?;
            info!("successfully deleted items from Radarr: {movies:?}");
            self.download_client.delete(&download_ids).await?;
        } else {
            info!(
                "no items will be deleted as no `--force-delete` flag is provided. Listing them instead: {movies:?}"
            );
            self.download_client.list(&download_ids).await?;
        }

        Ok(())
    }

    async fn watched_items(&self, user_id: &UserId) -> anyhow::Result<Vec<Item>> {
        let items = self
            .jellyfin
            .items(
                ItemsFilter::watched()
                    .user_id(user_id.as_ref())
                    .include_item_types(&["Movie", "Video"]),
            )
            .await?;

        let Some(retention_period) = self.retention_period else {
            if !items.is_empty() {
                warn!("no retention period is set for Radarr, will delete all movies immediately");
            }
            return Ok(items);
        };
        let retention_date = chrono::Utc::now() - retention_period;
        let mut safe_to_delete_items = vec![];

        for item in items {
            if let Some(last_played) = item
                .user_data
                .as_ref()
                .and_then(|user_data| user_data.last_played_date)
            {
                if retention_date > last_played {
                    safe_to_delete_items.push(item);
                } else {
                    debug!(
                        "retention period for \"{}\" is not yet passed ({} left), skipping",
                        item.name,
                        utils::retention_str(&last_played, &retention_date)
                    );
                }
            };
        }
        Ok(safe_to_delete_items)
    }

    /// delete movies with given ids
    async fn delete_movies(&self, movies_ids: &HashSet<u64>) -> anyhow::Result<()> {
        let delete_futs = movies_ids
            .iter()
            .map(|id| self.radarr_client.delete_movie(*id));
        let _ = futures::future::try_join_all(delete_futs).await?;
        Ok(())
    }

    /// finds Radarr's movie IDs for a given set of items and returns the ones
    /// that are safe to delete
    async fn movies_for_deletion(&self, items: &[Item]) -> Result<Vec<Movie>, anyhow::Error> {
        let tmdb_ids: Vec<_> = items.iter().filter_map(Item::tmdb_id).collect();
        let forbidden_tags = self.forbidden_tags().await?;
        let movies_futs = tmdb_ids
            .iter()
            .map(|id| self.filter_movies(id, &forbidden_tags));

        // get flattened list of ids
        let movies = futures::future::try_join_all(movies_futs)
            .await?
            .into_iter()
            .flat_map(Vec::into_iter)
            .collect();
        Ok(movies)
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
        debug!("forbidden movies tags configured: {:?}", self.tags_to_keep);

        let tags = self.radarr_client.tags().await?;
        let forbidden_tags = tags
            .iter()
            .filter(|t| self.tags_to_keep.contains(&t.label))
            .map(|t| t.id)
            .collect();

        debug!("forbidden tag ids: {forbidden_tags:?}");

        Ok(forbidden_tags)
    }

    /// gets movies by a given TMDB ID. Filters out movies that are safe to
    /// delete. A collection of movies is returned as there might be more than
    /// one file for a given TMDB ID
    async fn filter_movies(
        &self,
        tmdb_id: &str,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<Vec<Movie>> {
        let movies = self
            .radarr_client
            .movies_by_tmdb_id(tmdb_id)
            .await?
            .into_iter()
            .filter(|movie| safe_to_delete(movie, forbidden_tags))
            .collect();
        Ok(movies)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_movie_safe_to_delete() {
        let movie = Movie {
            title: "movie".to_string(),
            tags: Some(vec![1, 2, 3]),
            id: 1,
        };
        assert!(safe_to_delete(&movie, &[]));
    }

    #[test]
    fn test_movie_not_safe_to_delete_forbidden_tags() {
        let movie = Movie {
            title: "movie".to_string(),
            tags: Some(vec![5]),
            id: 1,
        };
        assert!(!safe_to_delete(&movie, &[4, 5, 6]));
    }
}
