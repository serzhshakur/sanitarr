use crate::{
    config::SonarrConfig,
    http::{Item, SeriesInfo, SonarrClient},
};
use log::{debug, info};
use std::collections::HashSet;

pub struct Sonarr {
    client: SonarrClient,
    tags_to_keep: Vec<String>,
}

impl Sonarr {
    pub fn new(config: &SonarrConfig) -> anyhow::Result<Self> {
        let client = SonarrClient::new(&config.base_url, &config.api_key)?;
        Ok(Self {
            client,
            tags_to_keep: config.tags_to_keep.clone(),
        })
    }

    async fn forbidden_tags(&self) -> anyhow::Result<Vec<u64>> {
        debug!("forbidden tags configured: {:?}", self.tags_to_keep);

        let tags = self.client.tags().await?;
        let forbidden_tags = tags
            .iter()
            .filter(|t| self.tags_to_keep.contains(&t.label))
            .map(|t| t.id)
            .collect();

        debug!("forbidden tag ids: {forbidden_tags:?}");

        Ok(forbidden_tags)
    }

    /// get all series IDs for a given TVDB ID
    async fn series_ids(
        &self,
        tvdb_id: &str,
        forbidden_tags: &[u64],
    ) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .client
            .series_by_tvdb_id(tvdb_id)
            .await?
            .iter()
            .filter_map(|series| safe_to_delete(series, forbidden_tags).then_some(series.id))
            .collect();
        Ok(ids)
    }

    /// query Sonarr history for given series ids and get download_id for each
    async fn download_ids(&self, ids: &HashSet<u64>) -> anyhow::Result<HashSet<String>> {
        let records = self.client.history_recods(ids).await?;
        let download_ids = records
            .into_iter()
            .filter_map(|r| r.download_id)
            .collect::<HashSet<String>>();
        Ok(download_ids)
    }

    /// get the history for a list of series IDs and delete them
    pub async fn delete_and_get_download_ids(
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
            let delete_futs = ids.iter().map(|id| self.client.delete_series(*id));
            let _ = futures::future::try_join_all(delete_futs).await?;
            let items: Vec<&String> = items.iter().map(|i| &i.name).collect::<Vec<&String>>();
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
        debug!("series {} has forbidden tags, skipping", series.title);
        return false;
    }
    if series.statistics.size_on_disk == 0 {
        debug!("series {} not present on disk, skipping", series.title);
        return false;
    }
    let Some(seasons) = &series.seasons else {
        debug!("series {} has no seasons, skipping", series.title);
        return false;
    };
    seasons.iter().all(|season| {
        let stats = &season.statistics;
        let fully_downloaded = stats.episode_file_count == stats.total_episode_count;
        let wont_air = season.monitored && stats.next_airing.is_none();
        let not_interested = !season.monitored && stats.episode_file_count == 0;

        fully_downloaded || wont_air || not_interested
    })
}
