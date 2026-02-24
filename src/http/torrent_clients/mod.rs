pub mod deluge;
pub mod qbittorrent;

use async_trait::async_trait;
use std::collections::HashSet;

use crate::config::TorrentRetentionConfig;

#[async_trait]
pub trait TorrentClient {
    /// delete torrents by provided hashes and also delete the associated files
    async fn delete(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>>;

    /// list all torrents in the client by their hashes
    async fn list(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>>;

    /// indicates whether the client supports delayed deletion (i.e. marking
    /// torrents as watched and then deleting them later after retention
    /// criteria are met)
    fn is_delayed_deletion_supported(&self) -> bool {
        false
    }

    /// mark torrents as watched by their hashes, so they can be deleted later
    /// after retention criteria are met. It can take different forms in
    /// different clients, e.g. adding a "watched" tag.
    async fn mark_as_watched(&self, _hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
        anyhow::bail!("mark_as_watched operation is not implemented for this client");
    }

    /// list all torrents that are marked as watched (e.g. have "watched" tag)
    /// in the client
    async fn list_watched(&self) -> anyhow::Result<Vec<TorrentInfo>> {
        anyhow::bail!("list_watched operation is not implemented for this client");
    }
}

/// A generic struct that represents a torrent in any torrent client
#[cfg_attr(test, derive(Default))]
pub struct TorrentInfo {
    pub name: String,
    pub hash: String,
    pub ratio: f64,
    pub seed_time: std::time::Duration,
    pub tracker: String,
}

impl TorrentInfo {
    /// Checks if the torrent can be deleted according to the retention config
    pub fn ready_for_deletion(&self, retention_cfg: &TorrentRetentionConfig) -> bool {
        if !retention_cfg.trackers.is_empty()
            && !retention_cfg
                .trackers
                .iter()
                .any(|t| self.tracker.contains(t))
        {
            return true;
        }

        match (retention_cfg.min_ratio, retention_cfg.min_seed_time) {
            (None, None) => true,
            (None, Some(min_seed_time)) => self.seed_time >= min_seed_time,
            (Some(min_ratio), None) => self.ratio >= min_ratio,
            (Some(min_ratio), Some(min_seed_time)) => {
                self.ratio >= min_ratio || self.seed_time >= min_seed_time
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TorrentRetentionConfig;
    use std::time::Duration;

    fn base_torrent() -> TorrentInfo {
        TorrentInfo {
            name: "foo".to_string(),
            hash: "deadbeef".to_string(),
            ratio: 1.0,
            seed_time: Duration::from_secs(60 * 60),
            tracker: "tracker.example.com".to_string(),
        }
    }

    #[test]
    fn test_ready_for_deletion_if_tracker_list_non_empty_and_no_match() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(0.1),
            min_seed_time: Some(Duration::from_secs(1)),
            trackers: vec!["allowed.com".to_string()],
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_if_ratio_and_seed_time_missing_in_config() {
        let cfg = TorrentRetentionConfig {
            min_ratio: None,
            min_seed_time: None,
            trackers: vec![],
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_if_min_ratio_is_defined() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(1.0),
            ..Default::default()
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_if_min_seed_time_is_defined() {
        let cfg = TorrentRetentionConfig {
            min_seed_time: Some(Duration::from_secs(60 * 60)),
            ..Default::default()
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_with_both_defined_in_config() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(0.99999999999999),
            min_seed_time: Some(Duration::from_secs(60 * 59)),
            ..Default::default()
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_only_min_ratio_satisfies() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(0.99999999999999),
            min_seed_time: Some(Duration::from_secs(2 * 60 * 60)),
            ..Default::default()
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_ready_for_deletion_only_min_seed_time_satisfies() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(999.0),
            min_seed_time: Some(Duration::from_secs(60 * 60)),
            ..Default::default()
        };
        assert!(base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_not_ready_for_deletion_with_min_ratio_defined() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(1.00000001),
            ..Default::default()
        };
        assert!(!base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_not_ready_for_deletion_with_min_seed_time_defined() {
        let cfg = TorrentRetentionConfig {
            min_seed_time: Some(Duration::from_secs(60 * 60 + 1)),
            ..Default::default()
        };
        assert!(!base_torrent().ready_for_deletion(&cfg));
    }

    #[test]
    fn test_not_ready_for_deletion_with_both_defined_in_config() {
        let cfg = TorrentRetentionConfig {
            min_ratio: Some(1.00001),
            min_seed_time: Some(Duration::from_secs(60 * 60 + 1)),
            ..Default::default()
        };
        assert!(!base_torrent().ready_for_deletion(&cfg));
    }
}
