use super::ResponseExt;
use anyhow::Ok;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, ClientBuilder, Url};
use serde::Deserialize;
use std::collections::HashSet;

/// A client for interacting with Radarr API.
/// https://radarr.video/docs/api/
pub struct RadarrClient {
    client: Client,
    base_url: Url,
}

impl RadarrClient {
    pub fn new(base_url: &str, api_key: &str) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(base_url)?;
        base_url.set_path("/api/v3/");

        let default_headers = auth_headers(api_key)?;
        let client = ClientBuilder::new()
            .default_headers(default_headers)
            .build()?;

        Ok(Self { client, base_url })
    }

    /// Get the movie IDs for a given TMDB ID.
    /// https://radarr.video/docs/api/#/Movie/get_api_v3_movie
    pub async fn movies_by_tmdb_id(&self, tmdb_id: &str) -> anyhow::Result<Vec<Movie>> {
        let url = self.base_url.join("movie")?;
        let response = self
            .client
            .get(url)
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
    pub async fn history_records(
        &self,
        movie_ids: &[u64],
    ) -> anyhow::Result<HashSet<HistoryRecord>> {
        let url = self.base_url.join("history")?;
        let mut query: Vec<(&str, u64)> = movie_ids.iter().map(|id| ("movieIds[]", *id)).collect();
        query.push(("pageSize", 100));

        let mut records = HashSet::new();
        let mut page = 1;

        loop {
            let history = self
                .client
                .get(url.clone())
                .query(&query)
                .query(&[("page", page)])
                .send()
                .await?
                .handle_error()
                .await?
                .json::<History>()
                .await?;

            if history.records.is_empty() {
                break;
            }
            records.extend(history.records);
            page += 1;
        }
        Ok(records)
    }

    /// Delete a movie by its ID and all associated files.
    /// https://radarr.video/docs/api/#/Movie/delete_api_v3_movie__id_
    pub async fn delete_movie(&self, movie_id: u64) -> anyhow::Result<()> {
        let url = self.base_url.join("movie/")?.join(&movie_id.to_string())?;
        self.client
            .delete(url)
            .query(&[("deleteFiles", "true")])
            .send()
            .await?
            .handle_error()
            .await?;
        Ok(())
    }

    /// Get all tags.
    pub async fn tags(&self) -> anyhow::Result<Vec<Tag>> {
        let url = self.base_url.join("tag")?;
        let response = self
            .client
            .get(url)
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}

fn auth_headers(api_key: &str) -> Result<HeaderMap, anyhow::Error> {
    let mut default_headers = HeaderMap::new();
    let mut header_value = HeaderValue::from_str(api_key)?;
    header_value.set_sensitive(true);
    default_headers.insert("x-api-key", header_value);
    Ok(default_headers)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Movie {
    pub title: String,
    pub id: u64,
    pub has_file: bool,
    pub tags: Option<Vec<u64>>,
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

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub label: String,
    pub id: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_auth_headers() {
        let headers = super::auth_headers("abc-key").unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("x-api-key").unwrap(), "abc-key");
    }
}
