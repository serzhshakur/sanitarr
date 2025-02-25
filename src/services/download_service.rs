use crate::{
    config::DownloadClientConfig,
    http::{DelugeClient, QbittorrentClient, TorrentClient, TorrentClientKind},
};
use log::{error, info};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// This is a high level service that interacts with Download client API and
/// transforms the data into a more usable format.
pub struct DownloadService {
    clients: HashMap<TorrentClientKind, GenericClient>,
    force_delete: bool,
}

type GenericClient = Box<dyn TorrentClient + Send + Sync>;

impl DownloadService {
    pub async fn new(
        configs: Vec<DownloadClientConfig>,
        force_delete: bool,
    ) -> anyhow::Result<Arc<Self>> {
        let mut clients: HashMap<TorrentClientKind, GenericClient> = HashMap::new();
        for cfg in configs {
            let (kind, client): (TorrentClientKind, GenericClient) = match cfg {
                DownloadClientConfig::Qbittorrent(cfg) => {
                    let client = QbittorrentClient::new(&cfg).await?;
                    (TorrentClientKind::Qbittorrent, Box::new(client))
                }
                DownloadClientConfig::Deluge(cfg) => {
                    let client = DelugeClient::new(&cfg).await?;
                    (TorrentClientKind::Deluge, Box::new(client))
                }
            };

            clients.insert(kind, client);
        }

        Ok(Arc::new(Self {
            clients,
            force_delete,
        }))
    }

    pub async fn delete(&self, hashes: HashSet<(TorrentClientKind, String)>) -> anyhow::Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }

        let mut per_client_hashes: HashMap<TorrentClientKind, HashSet<String>> = HashMap::new();
        for (kind, hash) in hashes {
            per_client_hashes.entry(kind).or_default().insert(hash);
        }

        for (kind, hashes) in per_client_hashes {
            match self.clients.get(&kind) {
                None => {
                    error!(
                        "unable to delete hashes {hashes:?}, no torrent client of kind \"{kind}\" defined in the system"
                    );
                }
                Some(client) => {
                    let torrents = client.list_torrents(&hashes).await?;
                    info!("found the following torrents for deletion: {torrents:?}");

                    if self.force_delete {
                        client.delete_torrents(&hashes).await?;
                        info!("deleted {} torrents", torrents.len());
                    } else if !torrents.is_empty() {
                        info!(
                            "no torrents will be deleted as no `--force-delete` flag is provided"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
