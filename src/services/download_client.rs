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

    pub async fn delete(&self, dry_run: bool, hashes: &[String]) -> anyhow::Result<Vec<String>> {
        let torrents = self.0.list_torrents(hashes).await?;
        for torrent in &torrents {
            info!("Deleting torrent: {torrent}");
        }

        if !dry_run {
            self.0.delete_torrents(hashes).await?;
            info!("Deleted {} torrents", torrents.len());
        }

        Ok(torrents)
    }
}
