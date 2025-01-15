use crate::services::{DownloadClient, Jellyfin, Sonarr};
use std::sync::Arc;

pub struct SeriesCleaner {
    radarr: Sonarr,
    jellyfin: Arc<Jellyfin>,
    download_client: Arc<DownloadClient>,
}

impl SeriesCleaner {
    pub fn new(
        sonarr: Sonarr,
        jellyfin: Arc<Jellyfin>,
        download_client: Arc<DownloadClient>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            radarr: sonarr,
            jellyfin,
            download_client,
        })
    }

    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let items = self.jellyfin.query_watched(&["Series"]).await?;
        if items.is_empty() {
            log::info!("no TV series found for deletion!");
            return Ok(());
        }
        let download_ids = self
            .radarr
            .delete_and_get_download_ids(force_delete, &items)
            .await?;

        self.download_client
            .delete(force_delete, &download_ids)
            .await?;

        Ok(())
    }
}
