mod deluge;
mod qbittorrent;

use async_trait::async_trait;
use std::collections::HashSet;

pub use deluge::DelugeClient;
pub use qbittorrent::QbittorrentClient;

#[async_trait]
pub trait TorrentClient {
    async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()>;

    async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<String>>;
}
