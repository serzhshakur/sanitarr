use crate::config::{DownloadClientsConfig, TorrentRetentionConfig};
use crate::http::{DelugeClient, QbittorrentClient, TorrentClient, TorrentInfo};
use log::{debug, error, info};
use serde::Deserialize;
use std::fmt::Display;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const DELUGE_NAME: &str = "Deluge";
const QBITTORRENT_NAME: &str = "qBittorrent";

/// This is a high level service that interacts with various Download clients,
/// that you define in a config file, through their API
#[derive(Clone)]
pub struct DownloadService {
    clients: Arc<HashMap<TorrentClientKind, GenericClient>>,
    retention_config: Arc<Option<crate::config::TorrentRetentionConfig>>,
}

type GenericClient = Box<dyn TorrentClient + Send + Sync>;

impl DownloadService {
    pub async fn new(
        cfg: DownloadClientsConfig,
        retention_config: Option<TorrentRetentionConfig>,
    ) -> anyhow::Result<Self> {
        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();

        if let Some(qbittorrent_cfg) = cfg.qbittorrent {
            let client = QbittorrentClient::new(&qbittorrent_cfg).await?;
            clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));
        }

        if let Some(deluge_cfg) = cfg.deluge {
            let client = DelugeClient::new(&deluge_cfg).await?;
            clients.insert(TorrentClientKind::Deluge, Box::new(client));
        }

        Ok(Self {
            clients: Arc::new(clients),
            retention_config: Arc::new(retention_config),
        })
    }

    /// queries each torrent client API and retrieves torrents names. Then
    /// writes the output to the log
    pub async fn list(
        &self,
        hashes: &HashMap<TorrentClientKind, HashSet<String>>,
    ) -> anyhow::Result<()> {
        for (kind, hashes) in hashes {
            let Some(client) = self.client(kind) else {
                error!("unable to list torrents {hashes:?}, no client \"{kind}\" is configured");
                continue;
            };
            let torrents = client.list(hashes).await?;
            let names: Vec<&str> = torrents.iter().map(|t| t.name.as_ref()).collect();
            info!("found the following torrents for deletion: {names:?}");
        }
        Ok(())
    }

    /// queries each torrent client API and deletes torrents by the given
    /// hashes.
    pub async fn delete(
        &self,
        hashes: &HashMap<TorrentClientKind, HashSet<String>>,
    ) -> anyhow::Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        for (kind, hashes) in hashes {
            let Some(client) = self.client(kind) else {
                error!("unable to delete torrents {hashes:?}, no client \"{kind}\" is configured");
                continue;
            };
            let torrents = client.list(hashes).await?;
            if torrents.is_empty() {
                debug!("no torrents to delete for a given client \"{kind}\", skipping");
                continue;
            }
            if client.is_delayed_deletion_supported() {
                let Some(retention_cfg) = self.retention_config.as_ref() else {
                    let names: Vec<&str> = torrents.iter().map(|t| t.name.as_ref()).collect();
                    client.delete(hashes).await?;
                    info!("deleted torrents {names:?} from \"{kind}\"");
                    continue;
                };
                let WatchedTorrents {
                    not_ready_to_delete,
                    ready_to_delete,
                } = WatchedTorrents::new(torrents, retention_cfg);

                let (deleted, marked_as_watched) = tokio::try_join!(
                    client.delete(&ready_to_delete),
                    client.mark_as_watched(&not_ready_to_delete)
                )?;

                let deleted_names = deleted
                    .iter()
                    .map(|t| t.name.as_ref())
                    .collect::<Vec<&str>>();

                let marked_as_watched_names = marked_as_watched
                    .iter()
                    .map(|t| t.name.as_ref())
                    .collect::<Vec<&str>>();

                info!("deleted torrents from \"{kind}\": {deleted_names:?}");
                info!("marked torrents as watched in \"{kind}\": {marked_as_watched_names:?}");
            } else {
                let names: Vec<&str> = torrents.iter().map(|t| t.name.as_ref()).collect();
                client.delete(hashes).await?;
                info!("deleted torrents {names:?} from \"{kind}\"");
            }
        }
        Ok(())
    }

    /// queries each torrent client API for "watched" torrents and deletes them
    /// if retention criteria are met
    pub async fn cleanup_watched(&self) -> anyhow::Result<()> {
        let Some(retention_cfg) = &*self.retention_config.clone() else {
            debug!("no torrents retention config defined, skipping delayed cleanup");
            return Ok(());
        };
        for (kind, client) in self.clients.clone().as_ref() {
            if !client.is_delayed_deletion_supported() {
                debug!("client {kind} doesn't support delayed deletion, skipping");
                continue;
            }
            let torrents = client.list_watched().await?;
            if torrents.is_empty() {
                debug!("no torrents marked as watched found for {kind}, skipping");
                continue;
            }
            let to_delete = WatchedTorrents::new(torrents, retention_cfg).ready_to_delete;
            if to_delete.is_empty() {
                debug!("no torrents found for deletion for {kind}, skipping");
                continue;
            }
            client.delete(&to_delete).await?;
        }
        Ok(())
    }

    fn client(&self, kind: &TorrentClientKind) -> Option<&GenericClient> {
        self.clients.get(kind)
    }
}

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

/// A struct that holds torrents that should either be deleted (if retention
/// config allows it) or just marked as watched for later deletion when
/// retention criteria are met
pub struct WatchedTorrents {
    not_ready_to_delete: HashSet<String>,
    ready_to_delete: HashSet<String>,
}

impl WatchedTorrents {
    pub fn new(torrents: Vec<TorrentInfo>, retention_cfg: &TorrentRetentionConfig) -> Self {
        torrents.into_iter().fold(
            Self {
                not_ready_to_delete: HashSet::new(),
                ready_to_delete: HashSet::new(),
            },
            |mut acc, torrent| {
                if torrent.ready_for_deletion(retention_cfg) {
                    acc.ready_to_delete.insert(torrent.hash);
                } else {
                    acc.not_ready_to_delete.insert(torrent.hash);
                }
                acc
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::{sync::Mutex, time::Duration};

    const LISTED_WATCHED: &[&str] = &["foo", "bar", "baz"];

    struct MockTorrentClient {
        listed: Arc<Mutex<HashSet<String>>>,
        marked_as_watched: Arc<Mutex<HashSet<String>>>,
        deleted: Arc<Mutex<HashSet<String>>>,
        listed_watched: Arc<Mutex<HashSet<String>>>,
        delayed_deletion_supported: bool,
    }

    impl MockTorrentClient {
        fn new() -> Self {
            Self {
                listed: Arc::new(Mutex::new(HashSet::new())),
                deleted: Arc::new(Mutex::new(HashSet::new())),
                marked_as_watched: Arc::new(Mutex::new(HashSet::new())),
                listed_watched: Arc::new(Mutex::new(HashSet::new())),
                delayed_deletion_supported: true,
            }
        }
    }

    #[async_trait]
    impl TorrentClient for MockTorrentClient {
        async fn list(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
            let mut listed_hashes = self.listed.lock().unwrap();
            listed_hashes.clear();
            listed_hashes.extend(hashes.clone());
            let response = hashes
                .iter()
                .map(|hash| TorrentInfo {
                    hash: hash.clone(),
                    ..Default::default()
                })
                .collect();
            Ok(response)
        }

        async fn delete(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
            let mut deleted_hashes = self.deleted.lock().unwrap();
            deleted_hashes.extend(hashes.clone());
            let response = hashes.iter().map(|_| Default::default()).collect();
            Ok(response)
        }

        async fn mark_as_watched(
            &self,
            hashes: &HashSet<String>,
        ) -> anyhow::Result<Vec<TorrentInfo>> {
            let mut marked_as_watched = self.marked_as_watched.lock().unwrap();
            marked_as_watched.extend(hashes.clone());
            let response = hashes.iter().map(|_| Default::default()).collect();
            Ok(response)
        }

        async fn list_watched(&self) -> anyhow::Result<Vec<TorrentInfo>> {
            let watched = LISTED_WATCHED
                .iter()
                .map(|h| TorrentInfo {
                    hash: h.to_string(),
                    name: h.to_string(),
                    ratio: 2.0,
                    ..Default::default()
                })
                .collect();

            let mut listed_watched = self.listed_watched.lock().unwrap();
            listed_watched.extend(LISTED_WATCHED.iter().map(|h| h.to_string()));

            Ok(watched)
        }

        fn is_delayed_deletion_supported(&self) -> bool {
            self.delayed_deletion_supported
        }
    }

    fn retention_cfg() -> TorrentRetentionConfig {
        TorrentRetentionConfig {
            min_ratio: Some(0.9),
            min_seed_time: Some(Duration::from_secs(60)),
            trackers: vec![],
        }
    }

    #[tokio::test]
    async fn test_download_service_delete_without_retention_cfg() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_hashes = client.listed.clone();
        let deleted_hashes = client.deleted.clone();
        let marked_as_watched = client.marked_as_watched.clone();
        let listed_watched = client.listed_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(None),
        };

        let listed = HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let listed_map = HashMap::from([(TorrentClientKind::Qbittorrent, listed.clone())]);

        service.list(&listed_map).await?;
        assert_eq!(*listed_hashes.lock().unwrap(), listed);

        let hashes = HashSet::from(["d".to_string(), "e".to_string(), "f".to_string()]);
        let hashes_per_client = HashMap::from([(TorrentClientKind::Qbittorrent, hashes.clone())]);

        service.delete(&hashes_per_client).await?;

        assert_eq!(*listed_hashes.lock().unwrap(), hashes);
        assert_eq!(*deleted_hashes.lock().unwrap(), hashes);
        assert_eq!(*marked_as_watched.lock().unwrap(), HashSet::new());
        assert_eq!(*listed_watched.lock().unwrap(), HashSet::new());

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_delete_with_retention_cfg() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_hashes = client.listed.clone();
        let deleted_hashes = client.deleted.clone();
        let marked_as_watched = client.marked_as_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(Some(retention_cfg())),
        };

        let hashes = HashSet::from(["d".to_string(), "e".to_string(), "f".to_string()]);
        let hashes_per_client = HashMap::from([(TorrentClientKind::Qbittorrent, hashes.clone())]);

        service.delete(&hashes_per_client).await?;

        assert_eq!(*listed_hashes.lock().unwrap(), hashes);
        assert_eq!(*deleted_hashes.lock().unwrap(), Default::default());
        assert_eq!(*marked_as_watched.lock().unwrap(), hashes);

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_delete_if_delayed_deletion_not_supported() -> anyhow::Result<()>
    {
        let mut client = MockTorrentClient::new();
        client.delayed_deletion_supported = false;

        let listed_hashes = client.listed.clone();
        let deleted_hashes = client.deleted.clone();
        let marked_as_watched = client.marked_as_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(Some(retention_cfg())),
        };

        let hashes = HashSet::from(["d".to_string(), "e".to_string(), "f".to_string()]);
        let hashes_per_client = HashMap::from([(TorrentClientKind::Qbittorrent, hashes.clone())]);

        service.delete(&hashes_per_client).await?;

        assert_eq!(*listed_hashes.lock().unwrap(), hashes);
        assert_eq!(*deleted_hashes.lock().unwrap(), hashes);
        assert_eq!(*marked_as_watched.lock().unwrap(), HashSet::new());

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_cleanup_watched_no_retention_cfg() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_watched = client.listed_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(None),
        };

        service.cleanup_watched().await?;
        assert_eq!(*listed_watched.lock().unwrap(), HashSet::new());

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_cleanup_watched_no_delayed_deletion_supported()
    -> anyhow::Result<()> {
        let mut client = MockTorrentClient::new();
        client.delayed_deletion_supported = false;

        let listed_watched = client.listed_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(Some(retention_cfg())),
        };

        service.cleanup_watched().await?;
        assert_eq!(*listed_watched.lock().unwrap(), HashSet::new());

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_cleanup_watched() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_watched = client.listed_watched.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(Some(retention_cfg())),
        };

        service.cleanup_watched().await?;

        let hashes: HashSet<String> = LISTED_WATCHED.iter().map(ToString::to_string).collect();
        assert_eq!(*listed_watched.lock().unwrap(), hashes);

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_undefined_client() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_hashes = client.listed.clone();
        let deleted_hashes = client.deleted.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService {
            clients: Arc::new(clients),
            retention_config: Arc::new(None),
        };

        let listed = HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let listed_map = HashMap::from([(TorrentClientKind::Deluge, listed)]);

        service.list(&listed_map).await?;
        assert!(listed_hashes.lock().unwrap().is_empty());

        let deleted = HashSet::from(["d".to_string(), "e".to_string(), "f".to_string()]);
        let deleted_map = HashMap::from([(TorrentClientKind::Deluge, deleted)]);

        service.delete(&deleted_map).await?;

        assert!(listed_hashes.lock().unwrap().is_empty());
        assert!(deleted_hashes.lock().unwrap().is_empty());

        Ok(())
    }

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

    #[test]
    fn test_torrent_info_ready_for_deletion() {
        let torrents = vec![
            TorrentInfo {
                hash: "a".to_string(),
                ratio: 0.01,
                ..Default::default()
            },
            TorrentInfo {
                hash: "b".to_string(),
                seed_time: Duration::from_secs(59),
                ..Default::default()
            },
            TorrentInfo {
                hash: "c".to_string(),
                ratio: 1.01,
                seed_time: Duration::from_secs(59),
                ..Default::default()
            },
            TorrentInfo {
                hash: "d".to_string(),
                ratio: 0.9999,
                seed_time: Duration::from_secs(60 * 2),
                ..Default::default()
            },
            TorrentInfo {
                hash: "e".to_string(),
                ratio: 0.0,
                seed_time: Duration::from_secs(1),
                tracker: "foo.bar".to_string(),
                ..Default::default()
            },
            TorrentInfo {
                hash: "f".to_string(),
                ratio: 2.0001,
                seed_time: Duration::from_secs(60),
                ..Default::default()
            },
        ];

        let retention_cfg = TorrentRetentionConfig {
            min_ratio: Some(1.0),
            min_seed_time: Some(Duration::from_secs(60)),
            trackers: vec![],
        };

        let WatchedTorrents {
            not_ready_to_delete,
            ready_to_delete,
        } = WatchedTorrents::new(torrents, &retention_cfg);

        let mut not_ready: Vec<&str> = not_ready_to_delete.iter().map(|t| t.as_str()).collect();
        let mut ready: Vec<&str> = ready_to_delete.iter().map(|t| t.as_str()).collect();

        not_ready.sort();
        ready.sort();

        assert_eq!(&ready, &["c", "d", "f"]);
        assert_eq!(&not_ready, &["a", "b", "e"]);
    }
}
