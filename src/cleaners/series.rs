use crate::{
    cleaners::utils,
    config::SonarrConfig,
    http::{
        Item, ItemsFilter, JellyfinClient, SeriesInfo, SonarrClient, TorrentClientKind, UserId,
    },
    services::DownloadService,
};
use log::{debug, info, warn};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

/// SeriesCleaner is responsible for cleaning up watched series from Sonarr and
/// Download client (e.g. qBittorrent).
pub struct SeriesCleaner {
    sonarr_client: SonarrClient,
    jellyfin: JellyfinClient,
    download_client: DownloadService,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
    unmonitor: bool,
}

impl SeriesCleaner {
    pub fn new(
        sonarr_config: SonarrConfig,
        jellyfin: JellyfinClient,
        download_client: DownloadService,
    ) -> anyhow::Result<Self> {
        let SonarrConfig {
            base_url,
            api_key,
            tags_to_keep,
            retention_period,
            unmonitor,
        } = sonarr_config;

        let sonarr_client = SonarrClient::new(&base_url, &api_key)?;
        Ok(Self {
            sonarr_client,
            jellyfin,
            download_client,
            tags_to_keep,
            retention_period,
            unmonitor,
        })
    }

    /// cleanup fully watched series from Sonarr and Download client
    pub async fn cleanup(&self, user_name: &str, force_delete: bool) -> anyhow::Result<()> {
        let user = self.jellyfin.user(user_name).await?;

        // Handle unmonitor functionality separately
        if self.unmonitor {
            self.handle_unmonitor(&user.id, force_delete).await?;
        }

        // Original deletion logic with retention period and tags
        let items = self.watched_items(user_name).await?;

        if items.is_empty() {
            log::info!("no fully watched series found!");
            return Ok(());
        }

        let series = self.series_for_deletion(&items).await?;

        if series.is_empty() {
            info!("no series found for deletion!");
            return Ok(());
        }

        let series_ids = series.iter().map(|s| s.id).collect::<HashSet<u64>>();
        let download_ids = self.download_ids(&series_ids).await?;

        if force_delete {
            debug!("trying to delete series {series:?}");
            self.delete_series(&series_ids).await?;
            info!("successfully deleted series: {series:?}");

            self.download_client.delete(&download_ids).await?;
        } else {
            info!(
                "no items will be deleted as no `--force-delete` flag is provided. Listing them instead: {series:?}"
            );
            self.download_client.list(&download_ids).await?;
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

    /// get all series from Sonarr for a given TVDB ID
    async fn series_for_tvdb_id(
        &self,
        tvdb_id: &str,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<Vec<SeriesInfo>> {
        let ids = self
            .sonarr_client
            .series_by_tvdb_id(tvdb_id)
            .await?
            .into_iter()
            .filter(|series| safe_to_delete(series, forbidden_tags))
            .collect();
        Ok(ids)
    }

    /// query Sonarr history for given series ids and get download_ids per each
    /// client kind for each
    async fn download_ids(
        &self,
        ids: &HashSet<u64>,
    ) -> anyhow::Result<HashMap<TorrentClientKind, HashSet<String>>> {
        let mut per_client_hashes = HashMap::new();
        let records = self.sonarr_client.history_records(ids).await?;
        for record in records {
            if let Some((kind, hash)) = record.download_id_per_client() {
                per_client_hashes
                    .entry(kind)
                    .or_insert_with(HashSet::new)
                    .insert(hash);
            }
        }
        Ok(per_client_hashes)
    }

    /// get all series ids for a given list of items that are safe to delete
    async fn series_for_deletion(&self, items: &[Item]) -> anyhow::Result<Vec<SeriesInfo>> {
        if items.is_empty() {
            return Ok(Default::default());
        }
        let tvdb_ids: Vec<&str> = items.iter().filter_map(Item::tvdb_id).collect();
        let forbidden_tags = self.forbidden_tags().await?;

        let futs = tvdb_ids
            .iter()
            .map(|id| self.series_for_tvdb_id(id, &forbidden_tags));

        let series = futures::future::try_join_all(futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<Vec<SeriesInfo>>();
        Ok(series)
    }

    /// Handle unmonitoring watched episodes in series
    async fn handle_unmonitor(&self, user_id: &UserId, force_delete: bool) -> anyhow::Result<()> {
        let watched_series = self.all_watched_series(user_id).await?;
        if watched_series.is_empty() {
            return Ok(());
        }

        let episodes_to_unmonitor = self
            .episodes_for_unmonitoring(&watched_series, user_id)
            .await?;
        if episodes_to_unmonitor.is_empty() {
            return Ok(());
        }

        if force_delete {
            debug!(
                "trying to unmonitor {} episodes in Sonarr",
                episodes_to_unmonitor.len()
            );
            self.sonarr_client
                .unmonitor_episodes(&episodes_to_unmonitor)
                .await?;
            info!(
                "successfully unmonitored {} episodes in Sonarr",
                episodes_to_unmonitor.len()
            );
        } else {
            info!(
                "dry run mode - {} episodes would be unmonitored",
                episodes_to_unmonitor.len()
            );
        }

        Ok(())
    }

    /// Get all series with at least one watched episode (for unmonitor)
    async fn all_watched_series(&self, user_id: &UserId) -> anyhow::Result<Vec<Item>> {
        // Get all series (not just fully watched ones)
        let all_series = self
            .jellyfin
            .items(
                ItemsFilter::new()
                    .user_id(user_id.as_ref())
                    .recursive()
                    .include_item_types(&["Series"])
                    .fields(&["ProviderIds"]),
            )
            .await?;

        let mut series_with_watched_episodes = Vec::new();

        // Check each series for watched episodes
        for series in all_series {
            let watched_episodes_filter = ItemsFilter::new()
                .user_id(user_id.as_ref())
                .recursive()
                .parent_id(&series.id)
                .include_item_types(&["Episode"])
                .played();

            let watched_episodes = self.jellyfin.items(watched_episodes_filter).await?;

            // If the series has at least one watched episode, include it
            if !watched_episodes.is_empty() {
                series_with_watched_episodes.push(series);
            }
        }

        Ok(series_with_watched_episodes)
    }

    /// Get episodes that should be unmonitored (watched episodes in watched series)
    async fn episodes_for_unmonitoring(
        &self,
        watched_series: &[Item],
        user_id: &UserId,
    ) -> anyhow::Result<HashSet<u64>> {
        let mut episodes_to_unmonitor = HashSet::new();

        for series_item in watched_series {
            // Get only watched episodes using the played filter directly from Jellyfin
            let watched_episodes_filter = ItemsFilter::new()
                .user_id(user_id.as_ref())
                .recursive()
                .parent_id(&series_item.id)
                .include_item_types(&["Episode"])
                .played();

            let watched_episodes = self.jellyfin.items(watched_episodes_filter).await?;
            // debug!(
            //     "found watched episodes to unmonitor: {}, in series {}, tvdb_id: {}",
            //     watched_episodes.len(),
            //     series_item.name,
            //     series_item.tvdb_id().unwrap_or("0")
            // );

            if watched_episodes.is_empty() {
                continue;
            }

            // Get series from Sonarr by TVDB ID
            if let Some(tvdb_id) = series_item.tvdb_id() {
                let sonarr_series = self.sonarr_client.series_by_tvdb_id(tvdb_id).await?;

                for series in sonarr_series {
                    // Get all episodes for this series from Sonarr
                    let sonarr_episodes =
                        self.sonarr_client.episodes_by_series_id(series.id).await?;

                    // Match watched Jellyfin episodes with Sonarr episodes and collect IDs
                    for watched_episode in &watched_episodes {
                        if let (Some(season_num), Some(episode_num)) = (
                            watched_episode.parent_index_number,
                            watched_episode.index_number,
                        ) {
                            // Find matching episode in Sonarr that is currently monitored
                            if let Some(sonarr_episode) = sonarr_episodes.iter().find(|ep| {
                                ep.season_number == season_num as u32
                                    && ep.episode_number == episode_num as u32
                                    && ep.monitored.unwrap_or(true) // Only include monitored episodes
                            }) {
                                episodes_to_unmonitor.insert(sonarr_episode.id);
                                debug!(
                                    "found watched episode to unmonitor: {} S{}E{} (ID: {})",
                                    series_item.name, season_num, episode_num, sonarr_episode.id
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(episodes_to_unmonitor)
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
