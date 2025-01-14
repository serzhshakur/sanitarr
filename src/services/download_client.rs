use crate::{config::DownloadClientConfig, http::QbittorrentClient};
use log::info;

pub struct DownloadClient(QbittorrentClient);

impl DownloadClient {
    pub async fn new(config: &DownloadClientConfig) -> anyhow::Result<Self> {
        match &config {
            DownloadClientConfig::Qbittorrent(c) => {
                let client = QbittorrentClient::new(&c.base_url, &c.username, &c.password).await?;
                Ok(Self(client))
            }
        }
    }

    pub async fn delete(&self, force_delete: bool, hashes: &[String]) -> anyhow::Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let torrents = self.0.list_torrents(hashes).await?;
        let torrent_paths = torrents.iter().map(|t| &t.content_path).collect::<Vec<_>>();

        info!("found the following torrents for deletion: {torrent_paths:?}");

        if force_delete {
            self.0.delete_torrents(hashes).await?;
            info!("deleted {} torrents", torrent_paths.len());
        }

        Ok(())
    }
}
