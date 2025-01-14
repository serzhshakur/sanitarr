use crate::{
    config::RadarrConfig,
    http::{jellyfin_client::Item, RadarrClient},
};
use std::collections::HashSet;

pub struct Radarr {
    client: RadarrClient,
}

impl Radarr {
    pub fn new(config: &RadarrConfig) -> anyhow::Result<Self> {
        let client = RadarrClient::new(&config.base_url, &config.api_key)?;
        Ok(Self { client })
    }

    /// Get the movie IDs for a given TMDB ID.
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

    /// Get the history for a list of movie IDs and delete them.
    pub async fn cleanup_and_get_download_ids(
        &self,
        items: &[Item],
    ) -> anyhow::Result<Vec<String>> {
        let tmdb_ids: Vec<&str> = items.iter().filter_map(|item| item.tmdb_id()).collect();
        let ids_futs = tmdb_ids.iter().map(|id| self.movie_ids(id));
        let ids = futures::future::try_join_all(ids_futs)
            .await?
            .into_iter()
            .flat_map(|i| i.into_iter())
            .collect::<Vec<u64>>();

        let history = self.client.get_history(&ids).await?;

        // let delete_futs = ids.iter().map(|id| self.client.delete_movie(*id));
        // let _ = futures::future::try_join_all(delete_futs).await?;

        let download_ids = history
            .records
            .into_iter()
            .filter_map(|r| r.download_id)
            .collect::<Vec<String>>();

        Ok(download_ids)
    }
}
