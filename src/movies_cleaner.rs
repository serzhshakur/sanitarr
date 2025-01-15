use crate::{
    http::{Item, ItemsFilter},
    services::{DownloadClient, Jellyfin, Radarr},
};
use std::sync::Arc;

pub struct MoviesCleaner {
    radarr: Radarr,
    jellyfin: Arc<Jellyfin>,
    download_client: Arc<DownloadClient>,
}

impl MoviesCleaner {
    pub fn new(
        radarr: Radarr,
        jellyfin: Arc<Jellyfin>,
        download_client: Arc<DownloadClient>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            radarr,
            jellyfin,
            download_client,
        })
    }

    async fn query_watched(&self) -> anyhow::Result<Vec<Item>> {
        let user_id = self.jellyfin.user_id().await?;
        let items_filter = ItemsFilter::new()
            .user_id(&user_id)
            .recursive()
            .played()
            .favorite(false)
            .include_item_types(&["Movie", "Video"])
            .fields(&["ProviderIds", "Path"]);
        self.jellyfin.query_items(items_filter).await
    }

    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let items = self.query_watched().await?;
        if items.is_empty() {
            log::info!("no movies found for deletion!");
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
