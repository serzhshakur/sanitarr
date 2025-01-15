use crate::{
    config::SonarrConfig,
    http::{Item, SonarrClient},
};
use log::{debug, info};
use std::collections::HashSet;

pub struct Sonarr {
    client: SonarrClient,
}

impl Sonarr {
    pub fn new(config: &SonarrConfig) -> anyhow::Result<Self> {
        let client = SonarrClient::new(&config.base_url, &config.api_key)?;
        Ok(Self { client })
    }

    /// get the series IDs for a given TVDB ID
    async fn series_ids(&self, tvdb_id: &str) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .client
            .series_by_tvdb_id(tvdb_id)
            .await?
            .iter()
            .filter_map(|m| m.present_on_disk().then_some(m.id))
            .collect();
        Ok(ids)
    }

    /// query Sonarr history for given series ids and get download_id for each
    async fn download_ids(&self, ids: &[u64]) -> anyhow::Result<HashSet<String>> {
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
        let ids_futs = tvdb_ids.iter().map(|id| self.series_ids(id));
        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<Vec<u64>>();

        let download_ids = self.download_ids(&ids).await?;

        if force_delete {
            debug!("attempting to delete series items {ids:?}");
            let delete_futs = ids.iter().map(|id| self.client.delete_series(*id));
            let _ = futures::future::try_join_all(delete_futs).await?;
            let items = items.iter().map(|i| &i.name).collect::<Vec<&String>>();
            info!("successfully deleted series: {items:?}");
        }

        Ok(download_ids)
    }
}
