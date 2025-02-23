use crate::{
    config::DownloadClientConfig,
    http::{DelugeClient, QbittorrentClient, TorrentClient},
};
use log::info;
use std::{collections::HashSet, sync::Arc};

/// This is a high level service that interacts with Download client API and
/// transforms the data into a more usable format.
pub struct DownloadService(GenericClient);

type GenericClient = Box<dyn TorrentClient + Send + Sync>;

impl DownloadService {
    pub async fn new(config: &DownloadClientConfig) -> anyhow::Result<Arc<Self>> {
        let client: GenericClient = match config {
            DownloadClientConfig::Qbittorrent(cfg) => {
                let client = QbittorrentClient::new(cfg).await?;
                Box::new(client)
            }
            DownloadClientConfig::Deluge(cfg) => {
                let client = DelugeClient::new(cfg).await?;
                Box::new(client)
            }
        };

        Ok(Arc::new(Self(client)))
    }

    pub async fn delete(&self, force_delete: bool, hashes: &HashSet<String>) -> anyhow::Result<()> {
        if !hashes.is_empty() {
            let torrents = self.0.list_torrents(hashes).await?;
            info!("found the following torrents for deletion: {torrents:?}");

            if force_delete {
                self.0.delete_torrents(hashes).await?;
                info!("deleted {} torrents", torrents.len());
            } else if !torrents.is_empty() {
                info!("no torrents will be deleted as no `--force-delete` flag is provided");
            }
        }
        Ok(())
    }
}
