use crate::{config::DownloadClientConfig, http::QbittorrentClient};
use log::info;
use std::{collections::HashSet, sync::Arc};

/// This is a high level service that interacts with Download client API and
/// transforms the data into a more usable format.
pub struct DownloadService(QbittorrentClient);

impl DownloadService {
    pub async fn new(config: &DownloadClientConfig) -> anyhow::Result<Arc<Self>> {
        match &config {
            DownloadClientConfig::Qbittorrent(cfg) => {
                let client = QbittorrentClient::new(cfg).await?;
                let it = Arc::new(Self(client));
                Ok(it)
            }
        }
    }

    pub async fn delete(&self, force_delete: bool, hashes: &HashSet<String>) -> anyhow::Result<()> {
        if !hashes.is_empty() {
            let torrents = self.0.list_torrents(hashes).await?;
            let torrents = torrents.iter().map(|t| &t.name).collect::<Vec<_>>();

            info!("found the following torrents for deletion: {torrents:?}");

            if force_delete {
                self.0.delete_torrents(hashes).await?;
                info!("deleted {} torrents", torrents.len());
            }
        }
        Ok(())
    }
}
