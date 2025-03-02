use crate::config::DownloadClientsConfig;
use crate::http::{DelugeClient, QbittorrentClient, TorrentClient, TorrentClientKind};
use log::{debug, error, info};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// This is a high level service that interacts with various Download clients,
/// that you define in a config file, through their API
#[derive(Clone)]
pub struct DownloadService(Arc<HashMap<TorrentClientKind, GenericClient>>);

type GenericClient = Box<dyn TorrentClient + Send + Sync>;

impl DownloadService {
    pub async fn new(cfg: DownloadClientsConfig) -> anyhow::Result<Self> {
        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();

        if let Some(qbittorrent_cfg) = cfg.qbittorrent {
            let client = QbittorrentClient::new(&qbittorrent_cfg).await?;
            clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));
        }

        if let Some(deluge_cfg) = cfg.deluge {
            let client = DelugeClient::new(&deluge_cfg).await?;
            clients.insert(TorrentClientKind::Deluge, Box::new(client));
        }

        Ok(Self(Arc::new(clients)))
    }

    /// queries each torrent client API and retrieves torrents names. Then
    /// writes the output to the log
    pub async fn list(
        &self,
        hashes: &HashMap<TorrentClientKind, HashSet<String>>,
    ) -> anyhow::Result<()> {
        for (kind, hashes) in hashes {
            let Some(client) = self.get_client(kind) else {
                error!("unable to list torrents {hashes:?}, no client \"{kind}\" is configured");
                continue;
            };
            let names = client.list_torrents(hashes).await?;
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
            let Some(client) = self.get_client(kind) else {
                error!("unable to delete torrents {hashes:?}, no client \"{kind}\" is configured");
                continue;
            };
            let names = client.list_torrents(hashes).await?;
            if names.is_empty() {
                debug!("no torrents to delete for a given client \"{kind}\", skipping");
            } else {
                client.delete_torrents(hashes).await?;
                info!("deleted torrents {names:?} from \"{kind}\"");
            }
        }
        Ok(())
    }

    fn get_client(&self, kind: &TorrentClientKind) -> Option<&GenericClient> {
        self.0.get(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockTorrentClient {
        listed_hashes: Arc<Mutex<HashSet<String>>>,
        deleted_hashes: Arc<Mutex<HashSet<String>>>,
    }

    impl MockTorrentClient {
        fn new() -> Self {
            Self {
                listed_hashes: Arc::new(Mutex::new(HashSet::new())),
                deleted_hashes: Arc::new(Mutex::new(HashSet::new())),
            }
        }
    }

    #[async_trait]
    impl TorrentClient for MockTorrentClient {
        async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<String>> {
            let mut listed_hashes = self.listed_hashes.lock().unwrap();
            listed_hashes.clear();
            listed_hashes.extend(hashes.clone());

            let response = ["foo", "bar", "baz"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect();
            Ok(response)
        }

        async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()> {
            let mut deleted_hashes = self.deleted_hashes.lock().unwrap();
            deleted_hashes.extend(hashes.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_download_service() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_hashes = client.listed_hashes.clone();
        let deleted_hashes = client.deleted_hashes.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService(Arc::new(clients));

        let listed = HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let listed_map = HashMap::from([(TorrentClientKind::Qbittorrent, listed.clone())]);

        service.list(&listed_map).await?;
        assert_eq!(*listed_hashes.lock().unwrap(), listed);

        let deleted = HashSet::from(["d".to_string(), "e".to_string(), "f".to_string()]);
        let deleted_map = HashMap::from([(TorrentClientKind::Qbittorrent, deleted.clone())]);

        service.delete(&deleted_map).await?;

        assert_eq!(*listed_hashes.lock().unwrap(), deleted);
        assert_eq!(*deleted_hashes.lock().unwrap(), deleted);

        Ok(())
    }

    #[tokio::test]
    async fn test_download_service_undefined_client() -> anyhow::Result<()> {
        let client = MockTorrentClient::new();
        let listed_hashes = client.listed_hashes.clone();
        let deleted_hashes = client.deleted_hashes.clone();

        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));

        let service = DownloadService(Arc::new(clients));

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
}
