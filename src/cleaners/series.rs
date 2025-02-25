use crate::{
    cleaners::utils,
    config::SonarrConfig,
    http::{Item, ItemsFilter, JellyfinClient, SeriesInfo, SonarrClient, TorrentClientKind},
    services::DownloadService,
};
use log::{debug, info, warn};
use std::{collections::HashSet, sync::Arc, time::Duration};

/// SeriesCleaner is responsible for cleaning up watched series from Sonarr and
/// Download client (e.g. qBittorrent).
pub struct SeriesCleaner {
    sonarr_client: SonarrClient,
    jellyfin: Arc<JellyfinClient>,
    download_client: Arc<DownloadService>,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
}

impl SeriesCleaner {
    pub fn new(
        sonarr_config: SonarrConfig,
        jellyfin: Arc<JellyfinClient>,
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
    pub async fn cleanup(&self, user_name: &str, force_delete: bool) -> anyhow::Result<()> {
        let items = self.watched_items(user_name).await?;

        if items.is_empty() {
            log::info!("no fully watched series found!");
            return Ok(());
        }

        let series_ids = self.series_ids_for_deletion(&items).await?;

        if series_ids.is_empty() {
            info!("no series found for deletion!");
            return Ok(());
        } else {
            debug!("found series ids for deletion {series_ids:?}");
        }

        let per_client_download_ids = self.download_ids(&series_ids).await?;

        if force_delete {
            debug!("attempting to delete series items {series_ids:?}");
            self.delete_series(&series_ids).await?;

            let names = items.iter().map(|i| &i.name);
            info!("successfully deleted series: {names:?}");

            self.download_client.delete(per_client_download_ids).await?;
        }

        Ok(())
    }

    async fn watched_items(&self, user_name: &str) -> anyhow::Result<Vec<Item>> {
        let user_id = self.jellyfin.user(user_name).await?.id;

        let items = self
            .jellyfin
            .items(
                ItemsFilter::watched()
                    .user_id(user_id.as_ref())
                    .include_item_types(&["Series"]),
            )
            .await?;

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
                .user_id(user_id.as_ref())
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
    async fn series_ids_for_tvdb_id(
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

    /// query Sonarr history for given series ids and get download_id and
    /// download client kind for each
    async fn download_ids(
        &self,
        ids: &HashSet<u64>,
    ) -> anyhow::Result<HashSet<(TorrentClientKind, String)>> {
        let records = self.sonarr_client.history_records(ids).await?;
        let per_client_download_ids = records
            .into_iter()
            .filter_map(|r| r.download_id_per_client())
            .collect();
        Ok(per_client_download_ids)
    }

    /// get all series ids for a given list of items that are safe to delete
    async fn series_ids_for_deletion(&self, items: &[Item]) -> anyhow::Result<HashSet<u64>> {
        if items.is_empty() {
            return Ok(HashSet::default());
        }
        let tvdb_ids: Vec<&str> = items.iter().filter_map(|item| item.tvdb_id()).collect();
        let forbidden_tags = self.forbidden_tags().await?;

        let ids_futs = tvdb_ids
            .iter()
            .map(|id| self.series_ids_for_tvdb_id(id, &forbidden_tags));

        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<HashSet<u64>>();
        Ok(ids)
    }

    /// delete series with given ids
    async fn delete_series(&self, series_ids: &HashSet<u64>) -> anyhow::Result<()> {
        let delete_futs = series_ids
            .iter()
            .map(|id| self.sonarr_client.delete_series(*id));
        let _ = futures::future::try_join_all(delete_futs).await?;
        Ok(())
    }
}

/// check if the series is safe to delete.
fn safe_to_delete(series: &SeriesInfo, forbidden_tags: &[u64]) -> bool {
    let has_forbidden_tags = series
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| forbidden_tags.contains(tag)));

    let title = &series.title;

    if has_forbidden_tags {
        debug!("{title}: series has forbidden tags, skipping");
        return false;
    }
    if series.statistics.size_on_disk == 0 {
        debug!("{title}: series not present on disk, skipping");
        return false;
    }
    let Some(seasons) = &series.seasons else {
        debug!("{title}: missing `seasons` entry, skipping");
        return false;
    };
    if seasons.is_empty() {
        debug!("{title}: series has no seasons, skipping");
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
