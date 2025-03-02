mod deluge;
mod qbittorrent;

use async_trait::async_trait;
use serde::Deserialize;
use std::{collections::HashSet, fmt::Display};

pub use deluge::DelugeClient;
pub use qbittorrent::QbittorrentClient;

#[async_trait]
pub trait TorrentClient {
    async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()>;
    async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<String>>;
}

const DELUGE_NAME: &str = "Deluge";
const QBITTORRENT_NAME: &str = "qBittorrent";

#[derive(Eq, Hash, PartialEq)]
pub enum TorrentClientKind {
    Deluge,
    Qbittorrent,
    Other(String),
}

impl<'de> Deserialize<'de> for TorrentClientKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "deluge" => Ok(Self::Deluge),
            "qbittorrent" => Ok(Self::Qbittorrent),
            _ => Ok(Self::Other(s)),
        }
    }
}

impl Display for TorrentClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TorrentClientKind::Deluge => DELUGE_NAME,
            TorrentClientKind::Qbittorrent => QBITTORRENT_NAME,
            TorrentClientKind::Other(s) => s,
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde() {
        #[derive(Deserialize)]
        struct Test {
            qbittorrent: TorrentClientKind,
            deluge: TorrentClientKind,
            other: TorrentClientKind,
        }

        let s = r#"{"qbittorrent":"qBittorrenT", "deluge":"deluge", "other":"foo"}"#;
        let test: Test = serde_json::from_str(s).unwrap();
        assert!(matches!(test.qbittorrent, TorrentClientKind::Qbittorrent));
        assert!(matches!(test.deluge, TorrentClientKind::Deluge));
        assert!(matches!(test.other, TorrentClientKind::Other(s) if s == "foo"));
    }
}
