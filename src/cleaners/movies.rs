use crate::{
    config::RadarrConfig,
    http::{Item, RadarrClient},
    services::{DownloadService, Jellyfin},
};
use log::{debug, info};
use std::{collections::HashSet, sync::Arc};

pub struct MoviesCleaner {
    radarr_client: RadarrClient,
    jellyfin: Arc<Jellyfin>,
    download_client: Arc<DownloadService>,
}

/// MoviesCleaner is responsible for cleaning up watched movies from Radarr and
/// Download client (e.g. qBittorrent).
impl MoviesCleaner {
    pub fn new(
        radarr_config: &RadarrConfig,
        jellyfin: Arc<Jellyfin>,
        download_client: Arc<DownloadService>,
    ) -> anyhow::Result<Self> {
        let radarr_client = RadarrClient::new(&radarr_config.base_url, &radarr_config.api_key)?;

        Ok(Self {
            radarr_client,
            jellyfin,
            download_client,
        })
    }

    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let items = self.jellyfin.query_watched(&["Movie", "Video"]).await?;
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

    /// get the movie IDs for a given TMDB ID
    async fn movie_ids(&self, tmdb_id: &str) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .radarr_client
            .movies_by_tmdb_id(tmdb_id)
            .await?
            .iter()
            .filter_map(|m| m.has_file.then_some(m.id))
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
        let ids_futs = tmdb_ids.iter().map(|id| self.movie_ids(id));
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
