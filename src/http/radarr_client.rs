use super::ResponseExt;
use anyhow::Ok;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::collections::HashSet;

pub struct RadarrClient {
    client: Client,
    base_url: Url,
    default_headers: HeaderMap,
}

impl RadarrClient {
    pub fn new(base_url: &str, api_key: &str) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(base_url)?;
        base_url.set_path("/api/v3/");

        let mut default_headers = HeaderMap::new();
        let mut header_value = HeaderValue::from_str(api_key)?;
        header_value.set_sensitive(true);
        default_headers.insert("x-api-key", header_value);

        Ok(Self {
            client: Client::new(),
            base_url,
            default_headers,
        })
    }

    /// Get the movie IDs for a given TMDB ID.
    /// https://radarr.video/docs/api/#/Movie/get_api_v3_movie
    pub async fn movies_by_tmdb_id(&self, tmdb_id: &str) -> anyhow::Result<Vec<Movie>> {
        let url = self.base_url.join("movie")?;
        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .query(&[("tmdbId", tmdb_id)])
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    /// Get the history for a list of movie IDs.
    /// https://radarr.video/docs/api/#/History/get_api_v3_history
    pub async fn get_history(&self, movie_ids: &[u64]) -> anyhow::Result<History> {
        let url = self.base_url.join("history")?;
        let mut query: Vec<(&str, u64)> = movie_ids.iter().map(|id| ("movieIds[]", *id)).collect();
        query.push(("pageSize", 200));

        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .query(&query)
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;

        Ok(response)
    }

    /// Delete a movie by its ID and all associated files.
    /// https://radarr.video/docs/api/#/Movie/delete_api_v3_movie__id_
    pub async fn delete_movie(&self, movie_id: u64) -> anyhow::Result<()> {
        let url = self.base_url.join("movie")?.join(&movie_id.to_string())?;
        self.client
            .delete(url)
            .headers(self.default_headers.clone())
            .query(&[("deleteFiles", "true")])
            .send()
            .await?
            .handle_error()
            .await?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Movie {
    pub id: u64,
    pub has_file: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub records: HashSet<HistoryRecord>,
}

#[derive(Deserialize, Debug, Hash, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub download_id: Option<String>,
}
