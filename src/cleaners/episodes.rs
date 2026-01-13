//! Episode-level deletion for TV shows from Sonarr.
//!
//! EpisodesCleaner monitors Jellyfin for watched episodes and deletes them from Sonarr
//! while preserving the series and unwatched episodes. This provides fine-grained control
//! over disk space usage compared to series-level deletion.
//!
//! # Deletion Workflow
//!
//! 1. Query Jellyfin for watched episodes matching retention period criteria
//! 2. Group episodes by TVDB ID (series identifier)
//! 3. For each series:
//!    - Check if series has forbidden tags (if found, skip entire series)
//!    - Fetch all episodes from Sonarr for the series
//!    - Match Jellyfin watched episodes to Sonarr episodes by season/episode number
//! 4. For each episode to delete:
//!    - **Unmonitor episode in Sonarr FIRST** (prevents re-download attempts)
//!    - Delete episode file from disk SECOND (safe now that it's unmonitored)
//!
//! # Critical Design Notes
//!
//! ## Episode Matching
//! Episodes are matched by season/episode number instead of episode ID because:
//! - Jellyfin episode IDs don't correspond to Sonarr episode IDs
//! - Season/episode numbers are reliable identifiers across systems
//!
//! Limitations:
//! - Special episodes (Season 0) may number differently between systems
//! - Absolute numbering (anime) may not map correctly to season/episode
//!
//! ## Unmonitor-First Design
//! Episodes MUST be unmonitored before file deletion to prevent race conditions:
//! - If file is deleted first: Sonarr sees episode as "missing" and re-downloads it
//! - If unmonitored first: Sonarr knows episode was intentionally removed
//!
//! This prevents wasted bandwidth and achieves the cleanup goal.
//!
//! # Field Requirements
//!
//! Jellyfin must provide these fields for episodes:
//! - `ProviderIds.Tvdb` - Series TVDB ID for Sonarr lookup
//! - `ParentIndexNumber` - Season number
//! - `IndexNumber` - Episode number
//! - `UserData.LastPlayedDate` - For retention period filtering
//!
//! If any required field is missing, the episode is skipped with a warning log.

use crate::{
    cleaners::utils,
    config::SonarrConfig,
    http::{EpisodeInfo, Item, ItemsFilter, JellyfinClient, SeriesInfo, SonarrClient},
    services::DownloadService,
};
use log::{debug, info, warn};
use std::{
    collections::HashMap,
    time::Duration,
};

/// EpisodesCleaner is responsible for cleaning up watched episodes from Sonarr.
/// Unlike SeriesCleaner which deletes entire series, EpisodesCleaner deletes
/// individual episodes while preserving unwatched episodes in the same series.
pub struct EpisodesCleaner {
    sonarr_client: SonarrClient,
    jellyfin: JellyfinClient,
    #[allow(dead_code)]
    download_client: DownloadService,
    tags_to_keep: Vec<String>,
    retention_period: Option<Duration>,
}

/// Track episodes to be deleted
struct EpisodeFileDeletion {
    series_title: String,
    season: u32,
    episode: u32,
    episode_id: u64,       // CRITICAL: Needed for unmonitoring
    episode_file_id: u64,  // Needed for file deletion
}

impl EpisodesCleaner {
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

    /// cleanup fully watched episodes from Sonarr
    pub async fn cleanup(&self, user_name: &str, force_delete: bool) -> anyhow::Result<()> {
        let episodes = self.watched_episodes(user_name).await?;

        if episodes.is_empty() {
            info!("no fully watched episodes found!");
            return Ok(());
        }

        let files_to_delete = self.episode_files_for_deletion(&episodes).await?;

        if files_to_delete.is_empty() {
            info!("no episode files found for deletion!");
            return Ok(());
        }

        if force_delete {
            debug!("deleting {} episode files", files_to_delete.len());
            let mut success_count = 0;
            let mut failure_count = 0;

            for file in &files_to_delete {
                match self.delete_single_episode(file).await {
                    Ok(_) => {
                        success_count += 1;
                        info!(
                            "deleted and unmonitored: {} S{:02}E{:02}",
                            file.series_title, file.season, file.episode
                        );
                    }
                    Err(e) => {
                        failure_count += 1;
                        log::error!(
                            "failed to delete {} S{:02}E{:02} (episode_id: {}, file_id: {}): {}",
                            file.series_title,
                            file.season,
                            file.episode,
                            file.episode_id,
                            file.episode_file_id,
                            e
                        );
                    }
                }
            }

            info!(
                "deletion complete: {} succeeded, {} failed",
                success_count, failure_count
            );

            if failure_count > 0 {
                return Err(anyhow::anyhow!(
                    "failed to delete {} out of {} episodes",
                    failure_count,
                    files_to_delete.len()
                ));
            }
        } else {
            info!("no items will be deleted as no `--force-delete` flag is provided. Listing them instead:");
            for file in &files_to_delete {
                info!("  - {} S{:02}E{:02}", file.series_title, file.season, file.episode);
            }
        }

        Ok(())
    }

    /// Delete a single episode: unmonitor FIRST, then delete file
    ///
    /// Order is critical: unmonitoring must happen before file deletion to prevent
    /// Sonarr from seeing the episode as "missing" and attempting to re-download it.
    async fn delete_single_episode(&self, file: &EpisodeFileDeletion) -> anyhow::Result<()> {
        // Step 1: CRITICAL - Unmonitor the episode FIRST to prevent re-download attempts
        // This tells Sonarr "this episode was intentionally removed, don't fetch it again"
        self.sonarr_client
            .unmonitor_episode(file.episode_id)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to unmonitor episode: {}",
                    e
                )
            })?;

        // Step 2: Delete the episode file from disk (safe now that it's unmonitored)
        self.sonarr_client
            .delete_episode_file(file.episode_file_id)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to delete episode file after unmonitoring: {}",
                    e
                )
            })?;

        Ok(())
    }

    /// Query Jellyfin for watched episodes with retention period filtering
    async fn watched_episodes(&self, user_name: &str) -> anyhow::Result<Vec<Item>> {
        let user_id = self.jellyfin.user(user_name).await?.id;

        // Query Jellyfin for watched Episode items (not Series)
        // Must explicitly request all needed fields: ProviderIds for series matching,
        // and season/episode numbers for episode matching
        let episodes = self
            .jellyfin
            .items(
                ItemsFilter::watched()
                    .user_id(user_id.as_ref())
                    .include_item_types(&["Episode"])
                    .fields(&["ProviderIds", "SeriesId", "SeriesName"]),
            )
            .await?;

        // Filter by retention period
        let Some(retention_period) = self.retention_period else {
            if !episodes.is_empty() {
                warn!("no retention period set, will delete episodes immediately");
            }
            return Ok(episodes);
        };

        let retention_date = chrono::Utc::now() - retention_period;
        let safe_to_delete = episodes
            .into_iter()
            .filter(|episode| {
                let should_delete = episode
                    .user_data
                    .as_ref()
                    .and_then(|ud| ud.last_played_date)
                    .map(|last_played| retention_date > last_played)
                    .unwrap_or(false);

                if !should_delete {
                    let last_played_str = episode
                        .user_data
                        .as_ref()
                        .and_then(|ud| ud.last_played_date)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_else(|| "unknown".to_string());
                    debug!(
                        "Episode {} not eligible for deletion (last played: {})",
                        episode.name, last_played_str
                    );
                }
                should_delete
            })
            .collect();

        Ok(safe_to_delete)
    }

    /// Get the list of forbidden tag IDs from Sonarr
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

    /// Check if series has any forbidden tags
    fn has_forbidden_tags(series: &SeriesInfo, forbidden_tags: &[u64]) -> bool {
        series
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| forbidden_tags.contains(tag)))
    }

    /// Map Jellyfin episodes to Sonarr episode files for deletion
    async fn episode_files_for_deletion(
        &self,
        jellyfin_episodes: &[Item],
    ) -> anyhow::Result<Vec<EpisodeFileDeletion>> {
        // Group episodes by TVDB ID (series identifier)
        let mut by_tvdb: HashMap<String, Vec<&Item>> = HashMap::new();
        for ep in jellyfin_episodes {
            if let Some(tvdb_id) = ep.tvdb_id() {
                by_tvdb
                    .entry(tvdb_id.to_string())
                    .or_insert_with(Vec::new)
                    .push(ep);
            } else {
                warn!(
                    "Episode '{}' has no TVDB ID, cannot match to Sonarr series, skipping",
                    ep.name
                );
            }
        }

        let mut files_to_delete = Vec::new();
        let forbidden_tags = self.forbidden_tags().await?;

        for (tvdb_id, jellyfin_episodes) in by_tvdb {
            // Get Sonarr series by TVDB ID
            let series_list = self.sonarr_client.series_by_tvdb_id(&tvdb_id).await?;

            for series in series_list {
                // Check if series has forbidden tags
                if Self::has_forbidden_tags(&series, &forbidden_tags) {
                    debug!(
                        "Series {} has forbidden tags, skipping all episodes",
                        series.title
                    );
                    continue;
                }

                // Get ALL episodes for this series from Sonarr
                let sonarr_episodes = self.sonarr_client.episodes_by_series(series.id).await?;

                // For each Jellyfin watched episode, find matching Sonarr episode
                for jellyfin_ep in &jellyfin_episodes {
                    let season = jellyfin_ep.season_number();
                    let episode = jellyfin_ep.episode_number();

                    // Validate that we have season and episode numbers
                    let (season_num, ep_num) = match (season, episode) {
                        (Some(s), Some(e)) => (s, e),
                        (None, Some(e)) => {
                            warn!(
                                "Episode '{}' from series {} missing season number, skipping",
                                jellyfin_ep.name, series.title
                            );
                            continue;
                        }
                        (Some(s), None) => {
                            warn!(
                                "Episode '{}' from series {} missing episode number, skipping",
                                jellyfin_ep.name, series.title
                            );
                            continue;
                        }
                        (None, None) => {
                            warn!(
                                "Episode '{}' from series {} missing both season and episode numbers, skipping",
                                jellyfin_ep.name, series.title
                            );
                            continue;
                        }
                    };

                    // Match by season and episode number (direct from Jellyfin!)
                    match sonarr_episodes
                        .iter()
                        .find(|se| se.season_number == season_num && se.episode_number == ep_num)
                    {
                        Some(sonarr_ep) => {
                            // Validate that episode has a file on disk before attempting deletion
                            match sonarr_ep.episode_file_id {
                                Some(file_id) => {
                                    files_to_delete.push(EpisodeFileDeletion {
                                        series_title: series.title.clone(),
                                        season: season_num,
                                        episode: ep_num,
                                        episode_id: sonarr_ep.id,
                                        episode_file_id: file_id,
                                    });
                                }
                                None => {
                                    debug!(
                                        "Episode {} S{:02}E{:02} has no file on disk in Sonarr, skipping",
                                        series.title, season_num, ep_num
                                    );
                                }
                            }
                        }
                        None => {
                            debug!(
                                "Episode {} S{:02}E{:02} not found in Sonarr, skipping",
                                series.title, season_num, ep_num
                            );
                        }
                    }
                }
            }
        }

        Ok(files_to_delete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_episode_file_deletion_formatting() {
        let deletion = EpisodeFileDeletion {
            series_title: "Breaking Bad".to_string(),
            season: 1,
            episode: 5,
            episode_id: 123,
            episode_file_id: 456,
        };

        let formatted = format!(
            "{} S{:02}E{:02}",
            deletion.series_title, deletion.season, deletion.episode
        );
        assert_eq!(formatted, "Breaking Bad S01E05");
    }
}
