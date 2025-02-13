use crate::{
    cleaners::utils,
    config::SonarrConfig,
    http::{Item, ItemsFilter, SeriesInfo, SonarrClient},
    services::{DownloadService, Jellyfin},
};
use log::{debug, info, warn};
use std::{collections::HashSet, sync::Arc, time::Duration};

/// SeriesCleaner is responsible for cleaning up watched series from Sonarr and
/// Download client (e.g. qBittorrent).
pub struct SeriesCleaner {
    sonarr_client: SonarrClient,
    jellyfin: Arc<Jellyfin>,
    download_client: Arc<DownloadService>,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
}

impl SeriesCleaner {
    pub fn new(
        sonarr_config: SonarrConfig,
        jellyfin: Arc<Jellyfin>,
        download_client: Arc<DownloadService>,
    ) -> anyhow::Result<Self> {
        let SonarrConfig {
            base_url,
            api_key,
            tags_to_keep,
            retention_period,
        } = sonarr_config;

        let sonarr_client = SonarrClient::new(&base_url, &api_key)?;
        Ok(Self {
            sonarr_client,
            jellyfin,
            download_client,
            tags_to_keep,
            retention_period,
        })
    }

    /// cleanup fully watched series from Sonarr and Download client
    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let items = self.watched_items().await?;

        if items.is_empty() {
            log::info!("no fully watched series found!");
            return Ok(());
        }
        let download_ids = self
            .delete_and_get_download_ids(force_delete, &items)
            .await?;

        self.download_client
            .delete(force_delete, &download_ids)
            .await?;

        Ok(())
    }

    async fn watched_items(&self) -> anyhow::Result<Vec<Item>> {
        let items = self.jellyfin.watched_items(&["Series"]).await?;
        let user_id = self.jellyfin.user_id().await?;

        let Some(retention_period) = self.retention_period else {
            if !items.is_empty() {
                warn!("no retention period is set for Sonarr, will delete all series immediately");
            }
            return Ok(items);
        };
        let retention_date = chrono::Utc::now() - retention_period;
        let mut safe_to_delete_items = vec![];

        for item in items {
            // Items of type "Episode" despite being watched sometime are not
            // being marked as played, so we need to build a separate filter for
            // them
            let filter = ItemsFilter::new()
                .user_id(&user_id)
                .recursive()
                .parent_id(&item.id)
                .include_item_types(&["Episode"]);

            let episodes = self.jellyfin.items(filter).await?;
            let max_last_played = episodes
                .iter()
                .filter_map(|episode| {
                    episode
                        .user_data
                        .as_ref()
                        .and_then(|user_data| user_data.last_played_date)
                })
                .max();

            if let Some(last_played) = max_last_played {
                if retention_date > last_played {
                    safe_to_delete_items.push(item);
                } else {
                    debug!(
                        "retention period for one or more episodes of \"{}\" is not yet passed ({} left), skipping",
                        item.name,
                        utils::retention_str(&last_played, &retention_date)
                    );
                }
            }
        }
        Ok(safe_to_delete_items)
    }

    async fn forbidden_tags(&self) -> anyhow::Result<Vec<u64>> {
        debug!("forbidden tags configured: {:?}", self.tags_to_keep);

        let tags = self.sonarr_client.tags().await?;
        let forbidden_tags = tags
            .iter()
            .filter(|t| self.tags_to_keep.contains(&t.label))
            .map(|t| t.id)
            .collect();

        debug!("forbidden tag ids: {forbidden_tags:?}");

        Ok(forbidden_tags)
    }

    /// get all series IDs from Sonarr for a given TVDB ID
    async fn series_ids(
        &self,
        tvdb_id: &str,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .sonarr_client
            .series_by_tvdb_id(tvdb_id)
            .await?
            .iter()
            .filter_map(|series| safe_to_delete(series, forbidden_tags).then_some(series.id))
            .collect();
        Ok(ids)
    }

    /// query Sonarr history for given series ids and get download_id for each
    async fn download_ids(&self, ids: &HashSet<u64>) -> anyhow::Result<HashSet<String>> {
        let records = self.sonarr_client.history_recods(ids).await?;
        let download_ids = records
            .into_iter()
            .filter_map(|r| r.download_id)
            .collect::<HashSet<String>>();
        Ok(download_ids)
    }

    /// get the history for a list of series IDs and delete them
    async fn delete_and_get_download_ids(
        &self,
        force_delete: bool,
        items: &[Item],
    ) -> anyhow::Result<HashSet<String>> {
        if items.is_empty() {
            return Ok(HashSet::default());
        }
        let tvdb_ids: Vec<&str> = items.iter().filter_map(|item| item.tvdb_id()).collect();
        let forbidden_tags = self.forbidden_tags().await?;

        let ids_futs = tvdb_ids
            .iter()
            .map(|id| self.series_ids(id, &forbidden_tags));

        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<HashSet<u64>>();

        if ids.is_empty() {
            info!("no series found for deletion!");
            return Ok(HashSet::default());
        } else {
            debug!("found series ids for deletion {ids:?}");
        }

        let download_ids = self.download_ids(&ids).await?;

        if force_delete {
            debug!("attempting to delete series items {ids:?}");
            let delete_futs = ids.iter().map(|id| self.sonarr_client.delete_series(*id));
            let _ = futures::future::try_join_all(delete_futs).await?;
            let items = items.iter().map(|i| &i.name);
            info!("successfully deleted series: {items:?}");
        }

        Ok(download_ids)
    }
}

/// check if the series is safe to delete.
fn safe_to_delete(series: &SeriesInfo, forbidden_tags: &[u64]) -> bool {
    let has_forbidden_tags = series
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| forbidden_tags.contains(tag)));

    if has_forbidden_tags {
        debug!("{}: series has forbidden tags, skipping", series.title);
        return false;
    }
    if series.statistics.size_on_disk == 0 {
        debug!("{}: series not present on disk, skipping", series.title);
        return false;
    }
    let Some(seasons) = &series.seasons else {
        debug!("{}: missing `seasons` entry, skipping", series.title);
        return false;
    };
    if seasons.is_empty() {
        debug!("{}: series has no seasons, skipping", series.title);
        return false;
    };
    seasons.iter().all(|season| {
        let stats = &season.statistics;
        let fully_downloaded = stats.episode_file_count >= stats.total_episode_count;
        let wont_air = stats.next_airing.is_none();
        fully_downloaded || wont_air
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Season, SeasonStatistics, SeriesStatistics};

    const FORBIDDEN_TAGS: &[u64] = &[4, 5, 6];

    #[test]
    fn test_safe_to_delete_as_fully_downloaded() {
        let season_1 = Season {
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: 1,
                total_episode_count: 1,
            },
        };
        let season_2 = Season {
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: 11,
                total_episode_count: 10,
            },
        };
        let series = SeriesInfo {
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: Some(vec![season_1, season_2]),
            ..Default::default()
        };

        assert!(safe_to_delete(&series, &[]));
    }

    #[test]
    fn test_safe_to_delete_as_wont_air() {
        let season = Season {
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: 1,
                total_episode_count: 10,
            },
        };
        let series = SeriesInfo {
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: Some(vec![season]),
            ..Default::default()
        };

        assert!(safe_to_delete(&series, &[]));
    }

    #[test]
    fn test_not_safe_to_delete_as_will_air() {
        let season_1 = Season {
            statistics: SeasonStatistics {
                next_airing: Some(Default::default()),
                episode_file_count: 1,
                total_episode_count: 2,
            },
        };
        let season_2 = Season {
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: 10,
                total_episode_count: 10,
            },
        };
        let series = SeriesInfo {
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: Some(vec![season_1, season_2]),
            ..Default::default()
        };

        assert!(!safe_to_delete(&series, &[]));
    }

    #[test]
    fn test_not_safe_to_delete_no_seasons() {
        let series = SeriesInfo {
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: None,
            ..Default::default()
        };

        assert!(!safe_to_delete(&series, FORBIDDEN_TAGS));
    }

    #[test]
    fn test_not_safe_to_delete_empty_seasons() {
        let series = SeriesInfo {
            tags: Some(vec![]),
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: Some(vec![]),
            ..Default::default()
        };

        assert!(!safe_to_delete(&series, &[]));
    }

    #[test]
    fn test_not_safe_to_delete_zero_size() {
        let series = SeriesInfo {
            statistics: SeriesStatistics { size_on_disk: 0 },
            seasons: Some(vec![]),
            ..Default::default()
        };

        assert!(!safe_to_delete(&series, FORBIDDEN_TAGS));
    }

    #[test]
    fn test_not_safe_to_delete_forbidden_tags() {
        let series = SeriesInfo {
            tags: Some(vec![1, 2, 3, 4]),
            statistics: SeriesStatistics { size_on_disk: 1 },
            seasons: Some(vec![]),
            ..Default::default()
        };

        assert!(!safe_to_delete(&series, FORBIDDEN_TAGS));
    }
}
