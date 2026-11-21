use crate::{
    cleaners::utils,
    config::SonarrConfig,
    http::{
        Episode, Item as JellyfinItem, ItemsFilter, JellyfinClient, Season, SeriesInfo,
        SonarrClient, TorrentClientKind, UserId,
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
    user_id: UserId,
    unmonitor_watched: bool,
    delete_by_season: bool,
}

impl SeriesCleaner {
    pub fn new(
        sonarr_config: SonarrConfig,
        jellyfin: JellyfinClient,
        download_client: DownloadService,
        user_id: &UserId,
    ) -> anyhow::Result<Self> {
        let SonarrConfig {
            base_url,
            api_key,
            tags_to_keep,
            retention_period,
            unmonitor_watched,
            delete_by_season,
        } = sonarr_config;

        let sonarr_client = SonarrClient::new(&base_url, &api_key)?;
        Ok(Self {
            sonarr_client,
            jellyfin,
            download_client,
            tags_to_keep,
            retention_period,
            user_id: user_id.clone(),
            unmonitor_watched,
            delete_by_season,
        })
    }

    /// unmonitor watched episodes (if configured) and cleanup fully watched
    /// series from Sonarr and Download client
    pub async fn cleanup(&self, force_delete: bool) -> anyhow::Result<()> {
        let series_with_watched_eps = self.shows_with_watched_episodes().await?;

        if series_with_watched_eps.is_empty() {
            log::info!("no fully watched series found!");
            return Ok(());
        }
        if self.unmonitor_watched {
            self.unmonitor_watched_episodes(&series_with_watched_eps)
                .await?;
        }
        let forbidden_tags = self.forbidden_tags().await?;
        if self.delete_by_season {
            return self
                .cleanup_seasons(&series_with_watched_eps, &forbidden_tags, force_delete)
                .await;
        }
        let series_to_delete =
            series_with_watched_eps.series_for_deletion(self.retention_period, &forbidden_tags)?;

        if series_to_delete.is_empty() {
            info!("no series found for deletion!");
            return Ok(());
        }

        let series_ids = series_to_delete
            .iter()
            .map(|s| s.id)
            .collect::<HashSet<u64>>();
        let download_ids = self.download_ids(&series_ids).await?;

        if force_delete {
            debug!("trying to delete series {series_to_delete:?}");
            self.delete_series(&series_ids).await?;
            info!("successfully deleted series: {series_to_delete:?}");

            self.download_client.delete(&download_ids).await?;
        } else {
            info!(
                "no items will be deleted as no `--force-delete` flag is provided. Listing them instead: {series_to_delete:?}"
            );
            self.download_client.list(&download_ids).await?;
        }

        Ok(())
    }

    /// unmonitor watched episodes that are still monitored
    async fn unmonitor_watched_episodes(
        &self,
        shows: &ShowsWithWatchedEpisodes,
    ) -> anyhow::Result<()> {
        let per_series_ep_ids = shows.monitored_ep_ids_per_series();
        if per_series_ep_ids.is_empty() {
            debug!("no monitored episodes found for unmonitoring");
        } else {
            let ids: HashSet<u64> = per_series_ep_ids.keys().copied().collect();
            let res = self.sonarr_client.unmonitor_episodes(&ids).await?;
            let log_msg = res
                .iter()
                .filter_map(|e| {
                    per_series_ep_ids
                        .get(&e.id)
                        .map(|series_title| format!("  - \"{series_title}\" {e}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
            info!("unmonitored episodes:\n{log_msg}");
        }
        Ok(())
    }

    async fn cleanup_seasons(
        &self,
        shows: &ShowsWithWatchedEpisodes,
        forbidden_tags: &[u64],
        force_delete: bool,
    ) -> anyhow::Result<()> {
        let retention_date = self.retention_period.map(|p| chrono::Utc::now() - p);
        let mut found = false;

        for show in &shows.0 {
            if has_forbidden_tags(&show.sonarr_series, forbidden_tags) {
                debug!(
                    "\"{}\": series has forbidden tags, skipping",
                    show.sonarr_series.title
                );
                continue;
            }
            let Some(seasons) = &show.sonarr_series.seasons else {
                debug!(
                    "\"{}\": missing `seasons` entry, skipping",
                    show.sonarr_series.title
                );
                continue;
            };

            let on_disk: Vec<_> = show
                .all_sonarr_episodes
                .iter()
                .filter(|ep| ep.has_file)
                .cloned()
                .collect();
            let last_played = season_last_played(&show.watched_jellyfin_episodes);
            let seasons_to_delete =
                finished_seasons(seasons, &on_disk, &last_played, retention_date);
            if seasons_to_delete.is_empty() {
                continue;
            }
            found = true;

            let episode_ids: HashSet<u64> = on_disk
                .iter()
                .filter(|ep| seasons_to_delete.contains(&ep.season_number))
                .map(|ep| ep.id)
                .collect();
            let files = self
                .sonarr_client
                .episode_files_by_series_id(show.sonarr_series.id)
                .await?;
            let file_ids: Vec<u64> = files
                .iter()
                .filter(|f| f.episode_id.is_some_and(|id| episode_ids.contains(&id)))
                .map(|f| f.id)
                .collect();

            let title = &show.sonarr_series.title;
            if force_delete {
                debug!("trying to delete files {file_ids:?} of \"{title}\"");
                let delete_futs = file_ids
                    .iter()
                    .map(|id| self.sonarr_client.delete_episode_file(*id));
                let _ = futures::future::try_join_all(delete_futs).await?;
                info!("successfully deleted files of seasons {seasons_to_delete:?} of \"{title}\"");
            } else {
                info!(
                    "no items will be deleted as no `--force-delete` flag is provided. Listing instead: seasons {seasons_to_delete:?} of \"{title}\" ({} files)",
                    file_ids.len()
                );
            }
        }

        if !found {
            info!("no seasons found for deletion!");
        }
        Ok(())
    }

    async fn shows_with_watched_episodes(&self) -> anyhow::Result<ShowsWithWatchedEpisodes> {
        // first query all watched episodes
        let mut watched_episodes = self
            .jellyfin
            .items(
                ItemsFilter::watched()
                    .user_id(self.user_id.as_ref())
                    .include_item_types(&["Episode"]),
            )
            .await?;

        let series_ids: HashSet<&str> = watched_episodes
            .iter()
            .filter_map(|ep| ep.series_id.as_deref())
            .collect();

        // then query all series for those episodes. Note that some series may
        // not be fully watched yet
        let series = self
            .jellyfin
            .items(
                ItemsFilter::new()
                    .user_id(self.user_id.as_ref())
                    .ids(series_ids.iter().copied().collect::<Vec<&str>>().as_slice())
                    .include_item_types(&["Series"])
                    .fields(&["ProviderIds"]),
            )
            .await?;

        // group watched episodes per series
        let mut episodes_per_series = Vec::with_capacity(series.len());
        for s in series {
            let (kept, removed) = watched_episodes
                .into_iter()
                .partition(|ep| ep.series_id.as_deref() == Some(s.id.as_str()));
            watched_episodes = removed;
            episodes_per_series.push((s, kept));
        }

        let futs = episodes_per_series.into_iter().map(
            |(jellyfin_series, jellyfin_episodes)| async move {
                let series_name = &jellyfin_series.name;
                let Some(tvdb_id) = jellyfin_series.tvdb_id() else {
                    warn!("series \"{series_name}\" has no TVDB id, skipping");
                    return Ok::<_, anyhow::Error>(None);
                };

                let Some(sonarr_series) =
                // assuming there is only one Sonarr series per TVDB id for
                // simplicity sake
                    self.sonarr_client.series_by_tvdb_id(tvdb_id).await?.pop()
                else {
                    warn!("series {series_name} with TVDB id {tvdb_id} not found in Sonarr");
                    return Ok::<_, anyhow::Error>(None);
                };
                let mut sonarr_episodes = self
                    .sonarr_client
                    .episodes_by_series_id(sonarr_series.id)
                    .await?;
                let all_sonarr_episodes = sonarr_episodes.clone();

                // retain only those Sonarr episodes that are watched in Jellyfin
                sonarr_episodes.retain(|sonar_ep| {
                    jellyfin_episodes.iter().any(|jl_ep| {
                        if !jl_ep.watched() {
                            return false;
                        }
                        let (Some(season_nr), Some(ep_nr)) =
                            (jl_ep.parent_index_number, jl_ep.index_number)
                        else {
                            return false;
                        };
                        sonar_ep.season_number == season_nr && sonar_ep.episode_number == ep_nr
                    })
                });

                let result = TvShowWithWatchedEpisodes {
                    jellyfin_series,
                    watched_jellyfin_episodes: jellyfin_episodes,
                    sonarr_series,
                    all_sonarr_episodes,
                    watched_sonarr_episodes: sonarr_episodes,
                };
                Ok::<_, anyhow::Error>(Some(result))
            },
        );

        let results = futures::future::try_join_all(futs)
            .await?
            .into_iter()
            .flatten()
            .collect();

        Ok(ShowsWithWatchedEpisodes(results))
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
    let title = &series.title;

    if has_forbidden_tags(series, forbidden_tags) {
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

fn has_forbidden_tags(series: &SeriesInfo, forbidden_tags: &[u64]) -> bool {
    series
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| forbidden_tags.contains(tag)))
}

fn season_last_played(
    watched_episodes: &[JellyfinItem],
) -> HashMap<(u32, u32), chrono::DateTime<chrono::Utc>> {
    watched_episodes
        .iter()
        .filter(|ep| ep.watched())
        .filter_map(|ep| {
            let (Some(season_nr), Some(ep_nr), Some(date)) = (
                ep.parent_index_number,
                ep.index_number,
                ep.last_played_date(),
            ) else {
                return None;
            };
            Some(((season_nr, ep_nr), date))
        })
        .collect()
}

fn finished_seasons(
    seasons: &[Season],
    episodes: &[Episode],
    last_played: &HashMap<(u32, u32), chrono::DateTime<chrono::Utc>>,
    retention_date: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<u32> {
    let mut result = Vec::new();
    'season: for season in seasons {
        let stats = &season.statistics;
        if stats.total_episode_count == 0
            || stats.episode_file_count < stats.total_episode_count
            || stats.next_airing.is_some()
        {
            continue;
        }

        let season_episodes: Vec<_> = episodes
            .iter()
            .filter(|ep| ep.season_number == season.season_number)
            .collect();
        if season_episodes.is_empty() {
            continue;
        }

        let mut latest_played: Option<chrono::DateTime<chrono::Utc>> = None;
        for ep in season_episodes {
            let Some(date) = last_played.get(&(ep.season_number, ep.episode_number)) else {
                debug!(
                    "season {} is not fully watched, skipping",
                    season.season_number
                );
                continue 'season;
            };
            latest_played = Some(latest_played.map_or(*date, |d| d.max(*date)));
        }

        if let (Some(retention_date), Some(latest_played)) = (retention_date, latest_played)
            && retention_date <= latest_played
        {
            debug!(
                "retention period for season {} is not yet passed, skipping",
                season.season_number
            );
            continue;
        }

        result.push(season.season_number);
    }
    result
}

/// a struct that represents a TV show with only watched Jellyfin episodes and
/// the corresponding episodes in Sonarr. Here the TV show itself is not
/// necessarily fully watched
struct TvShowWithWatchedEpisodes {
    jellyfin_series: JellyfinItem,
    watched_jellyfin_episodes: Vec<JellyfinItem>,
    sonarr_series: SeriesInfo,
    all_sonarr_episodes: Vec<Episode>,
    watched_sonarr_episodes: Vec<Episode>,
}

impl TvShowWithWatchedEpisodes {
    fn latest_played_date(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.watched_jellyfin_episodes
            .iter()
            .filter_map(JellyfinItem::last_played_date)
            .max()
    }
}

/// a collection of [`TvShowWithWatchedEpisodes`] with some helper methods
struct ShowsWithWatchedEpisodes(Vec<TvShowWithWatchedEpisodes>);

impl ShowsWithWatchedEpisodes {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// get a map of monitored episode ids per series titles. Needed for further
    /// logging to map episode ids back to series titles
    fn monitored_ep_ids_per_series(&self) -> HashMap<u64, &str> {
        self.0
            .iter()
            .flat_map(|s| {
                s.watched_sonarr_episodes
                    .iter()
                    .map(|ep| (s.jellyfin_series.name.as_ref(), ep))
            })
            .filter_map(|(title, ep)| ep.monitored.then_some((ep.id, title)))
            .collect()
    }

    /// get all Sonarr series from the collection
    fn sonar_series(&self) -> Vec<&SeriesInfo> {
        self.0.iter().map(|s| &s.sonarr_series).collect()
    }

    /// filter series that are safe to delete based on retention period and
    /// forbidden tags
    fn series_for_deletion(
        &self,
        retention_period: Option<Duration>,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<Vec<&SeriesInfo>> {
        let series = match retention_period {
            Some(retention_period) => {
                let retention_date = chrono::Utc::now() - retention_period;
                let mut safe_to_delete_items = vec![];

                for item in &self.0 {
                    if !item.jellyfin_series.watched() {
                        continue;
                    }
                    if let Some(last_played) = item.latest_played_date() {
                        if retention_date > last_played {
                            safe_to_delete_items.push(&item.sonarr_series);
                        } else {
                            debug!(
                                "retention period for one or more episodes of \"{}\" is not yet passed ({} left), skipping",
                                item.sonarr_series.title,
                                utils::retention_str(&last_played, &retention_date)
                            );
                        }
                    };
                }
                safe_to_delete_items
            }
            None => {
                if !self.0.is_empty() {
                    warn!(
                        "no retention period is set for Radarr, will delete all movies immediately"
                    );
                }
                self.sonar_series()
            }
        };

        let result = series
            .into_iter()
            .filter(|s| safe_to_delete(s, forbidden_tags))
            .collect();

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Season, SeasonStatistics, SeriesStatistics};

    const FORBIDDEN_TAGS: &[u64] = &[4, 5, 6];

    #[test]
    fn test_safe_to_delete_as_fully_downloaded() {
        let season_1 = Season {
            season_number: 1,
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: 1,
                total_episode_count: 1,
            },
        };
        let season_2 = Season {
            season_number: 2,
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
            season_number: 1,
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
            season_number: 1,
            statistics: SeasonStatistics {
                next_airing: Some(Default::default()),
                episode_file_count: 1,
                total_episode_count: 2,
            },
        };
        let season_2 = Season {
            season_number: 2,
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

    fn ep(season_number: u32, episode_number: u32, id: u64) -> Episode {
        Episode {
            episode_number,
            id,
            monitored: true,
            season_number,
            has_file: true,
        }
    }

    fn season(season_number: u32, files: usize, total: usize) -> Season {
        Season {
            season_number,
            statistics: SeasonStatistics {
                next_airing: None,
                episode_file_count: files,
                total_episode_count: total,
            },
        }
    }

    fn watched_map(
        playback: &[((u32, u32), i64)],
    ) -> HashMap<(u32, u32), chrono::DateTime<chrono::Utc>> {
        playback
            .iter()
            .map(|(key, ago)| (*key, chrono::Utc::now() - chrono::Duration::days(*ago)))
            .collect()
    }

    fn week_ago() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() - chrono::Duration::days(7)
    }

    #[test]
    fn test_finished_season_fully_watched() {
        let seasons = vec![season(1, 2, 2)];
        let episodes = vec![ep(1, 1, 11), ep(1, 2, 12)];
        let last_played = watched_map(&[((1, 1), 30), ((1, 2), 20)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn test_finished_season_skips_unwatched_episode() {
        let seasons = vec![season(1, 2, 2)];
        let episodes = vec![ep(1, 1, 11), ep(1, 2, 12)];
        let last_played = watched_map(&[((1, 1), 30)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert!(result.is_empty());
    }

    #[test]
    fn test_finished_season_skips_retention_not_passed() {
        let seasons = vec![season(1, 2, 2)];
        let episodes = vec![ep(1, 1, 11), ep(1, 2, 12)];
        let last_played = watched_map(&[((1, 1), 30), ((1, 2), 3)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert!(result.is_empty());
    }

    #[test]
    fn test_finished_season_skips_incomplete_download() {
        let seasons = vec![season(1, 1, 2)];
        let episodes = vec![ep(1, 1, 11)];
        let last_played = watched_map(&[((1, 1), 30)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert!(result.is_empty());
    }

    #[test]
    fn test_finished_season_skips_upcoming_airing() {
        let mut seasons = vec![season(1, 2, 2)];
        seasons[0].statistics.next_airing = Some(chrono::Utc::now());
        let episodes = vec![ep(1, 1, 11), ep(1, 2, 12)];
        let last_played = watched_map(&[((1, 1), 30), ((1, 2), 30)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert!(result.is_empty());
    }

    #[test]
    fn test_finished_seasons_returns_only_finished_ones() {
        let seasons = vec![season(1, 2, 2), season(2, 1, 10)];
        let episodes = vec![ep(1, 1, 11), ep(1, 2, 12), ep(2, 1, 21)];
        let last_played = watched_map(&[((1, 1), 30), ((1, 2), 20), ((2, 1), 30)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, Some(week_ago()));
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn test_finished_season_no_retention_deletes_immediately() {
        let seasons = vec![season(1, 1, 1)];
        let episodes = vec![ep(1, 1, 11)];
        let last_played = watched_map(&[((1, 1), 0)]);

        let result = finished_seasons(&seasons, &episodes, &last_played, None);
        assert_eq!(result, vec![1]);
    }
}
