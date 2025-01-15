use crate::{
    config::RadarrConfig,
    http::{Item, RadarrClient},
};
use log::{debug, info};
use std::collections::HashSet;

pub struct Radarr {
    client: RadarrClient,
}

impl Radarr {
    pub fn new(config: &RadarrConfig) -> anyhow::Result<Self> {
        let client = RadarrClient::new(&config.base_url, &config.api_key)?;
        Ok(Self { client })
    }

    /// get the movie IDs for a given TMDB ID
    async fn movie_ids(&self, tmdb_id: &str) -> anyhow::Result<HashSet<u64>> {
        let ids = self
            .client
            .movies_by_tmdb_id(tmdb_id)
            .await?
            .iter()
            .filter_map(|m| m.has_file.then_some(m.id))
            .collect();
        Ok(ids)
    }

    /// query Radarr history for given movie ids and get download_id for each
    async fn download_ids(&self, ids: &[u64]) -> anyhow::Result<HashSet<String>> {
        let records = self.client.history_records(ids).await?;
        let download_ids = records.into_iter().filter_map(|r| r.download_id).collect();
        Ok(download_ids)
    }

    /// get the history for a list of movie IDs and delete them
    pub async fn delete_and_get_download_ids(
        &self,
        force_delete: bool,
        items: &[Item],
    ) -> anyhow::Result<HashSet<String>> {
        let tmdb_ids: Vec<&str> = items.iter().filter_map(|item| item.tmdb_id()).collect();
        let ids_futs = tmdb_ids.iter().map(|id| self.movie_ids(id));
        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<Vec<u64>>();

        if ids.is_empty() {
            info!("no movies found for deletion!");
            return Ok(HashSet::default());
        } else {
            debug!("found movie ids for deletion {ids:?}");
        }

        let download_ids = self.download_ids(&ids).await?;

        if force_delete {
            debug!("attempting to delete items {ids:?}");
            let delete_futs = ids.iter().map(|id| self.client.delete_movie(*id));
            let _ = futures::future::try_join_all(delete_futs).await?;
            let items = items.iter().map(|i| &i.name).collect::<Vec<&String>>();
            info!("successfully deleted items: {items:?}");
        }

        Ok(download_ids)
    }
}
