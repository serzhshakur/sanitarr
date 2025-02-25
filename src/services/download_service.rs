use crate::{
    config::DownloadClientsConfig,
    http::{DelugeClient, QbittorrentClient, TorrentClient, TorrentClientKind},
};
use log::{debug, error, info};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// This is a high level service that interacts with Download client API and
/// transforms the data into a more usable format.
pub struct DownloadService(HashMap<TorrentClientKind, GenericClient>);

type GenericClient = Box<dyn TorrentClient + Send + Sync>;

impl DownloadService {
    pub async fn new(cfg: DownloadClientsConfig) -> anyhow::Result<Arc<Self>> {
        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();

        if let Some(qbittorrent_cfg) = cfg.qbittorrent {
            let client = QbittorrentClient::new(&qbittorrent_cfg).await?;
            clients.insert(TorrentClientKind::Qbittorrent, Box::new(client));
        }

        if let Some(deluge_cfg) = cfg.deluge {
            let client = DelugeClient::new(&deluge_cfg).await?;
            clients.insert(TorrentClientKind::Deluge, Box::new(client));
        }

        Ok(Arc::new(Self(clients)))
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

    /// queries each torrent client API and deletes torrents.
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
