use crate::{
    config::RadarrConfig,
    http::{Item, Movie, RadarrClient},
    services::{DownloadService, Jellyfin},
};
use log::{debug, info};
use std::{collections::HashSet, sync::Arc, time::Duration};

pub struct MoviesCleaner {
    radarr_client: RadarrClient,
    jellyfin: Arc<Jellyfin>,
    download_client: Arc<DownloadService>,
    tags_to_keep: Vec<String>,
    retention_period: Duration,
}

/// MoviesCleaner is responsible for cleaning up watched movies from Radarr and
/// Download client (e.g. qBittorrent).
impl MoviesCleaner {
    pub fn new(
        radarr_config: RadarrConfig,
        jellyfin: Arc<Jellyfin>,
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

    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let items = self.watched_items().await?;
        if items.is_empty() {
            log::info!("no movies found for deletion in Jellyfin!");
            return Ok(());
        }
        let download_ids = self
            .delete_and_get_download_ids(force_delete, &items)
            .await?;

        self.download_client
            .delete(force_delete, &download_ids)
            .await?;

        Ok(())
    }

    async fn watched_items(&self) -> anyhow::Result<Vec<Item>> {
        let items = self.jellyfin.watched_items(&["Movie", "Video"]).await?;
        let retention_date = chrono::Utc::now() - self.retention_period;
        let mut safe_to_delete_items = vec![];

        for item in items {
            let old_enough = item
                .user_data
                .as_ref()
                .and_then(|user_data| user_data.last_played_date)
                .is_some_and(|last_played| retention_date > last_played);

            if old_enough {
                safe_to_delete_items.push(item);
            } else {
                debug!(
                    "last played date for '{}' is after retention date {}, skipping",
                    item.name,
                    retention_date.format("%Y-%m-%d %H:%M:%S")
                );
            }
        }
        Ok(safe_to_delete_items)
    }

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

    /// get the movie IDs for a given TMDB ID
    async fn movie_ids(
        &self,
        tmdb_id: &str,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .radarr_client
            .movies_by_tmdb_id(tmdb_id)
            .await?
            .iter()
            .filter_map(|movie| movie_safe_to_delete(movie, forbidden_tags).then_some(movie.id))
            .collect();
        Ok(ids)
    }

    /// query Radarr history for given movie ids and get download_id for each
    async fn download_ids(&self, ids: &[u64]) -> anyhow::Result<HashSet<String>> {
        let records = self.radarr_client.history_records(ids).await?;
        let download_ids = records.into_iter().filter_map(|r| r.download_id).collect();
        Ok(download_ids)
    }

    /// get the history for a list of movie IDs and delete them
    pub async fn delete_and_get_download_ids(
        &self,
        force_delete: bool,
        items: &[Item],
    ) -> anyhow::Result<HashSet<String>> {
        let tmdb_ids: Vec<&str> = items.iter().filter_map(|item| item.tmdb_id()).collect();

        let forbidden_tags = self.forbidden_tags().await?;
        let ids_futs = tmdb_ids
            .iter()
            .map(|id| self.movie_ids(id, &forbidden_tags));
        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<Vec<u64>>();

        if ids.is_empty() {
            info!("no movies found for deletion in Radarr!");
            return Ok(HashSet::default());
        } else {
            debug!("found movie ids for deletion {ids:?}");
        }

        let download_ids = self.download_ids(&ids).await?;

        if force_delete {
            debug!("trying to delete items in Radarr: {ids:?}");
            let delete_futs = ids.iter().map(|id| self.radarr_client.delete_movie(*id));
            let _ = futures::future::try_join_all(delete_futs).await?;
            let items = items.iter().map(|i| &i.name).collect::<Vec<&String>>();
            info!("successfully deleted items from Radarr: {items:?}");
        }

        Ok(download_ids)
    }
}

/// check if it's safe to delete a movie.
fn movie_safe_to_delete(movie: &Movie, forbidden_tags: &[u64]) -> bool {
    if movie.has_file {
        debug!("movie '{}' not present on disk, skipping", movie.title);
        return false;
    }

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
            has_file: false,
            tags: Some(vec![1, 2, 3]),
            id: 1,
        };

        assert!(movie_safe_to_delete(&movie, &[4, 5, 6]));
    }
}
