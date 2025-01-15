use super::ResponseExt;
use anyhow::Ok;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::collections::HashSet;

pub struct SonarrClient {
    client: Client,
    base_url: Url,
    default_headers: HeaderMap,
}

impl SonarrClient {
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

    /// Get the series IDs for a given TVDB ID.
    /// https://sonarr.tv/docs/api/#/Series/get_api_v3_series
    pub async fn series_by_tvdb_id(&self, provider_id: &str) -> anyhow::Result<Vec<SeriesInfo>> {
        let url = self.base_url.join("series")?;
        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .query(&[("tvdbId", provider_id)])
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    /// Get the history records for a list of series IDs.
    /// https://sonarr.tv/docs/api/#/History/get_api_v3_history
    pub async fn history_recods(
        &self,
        movie_ids: &HashSet<u64>,
    ) -> anyhow::Result<HashSet<HistoryRecord>> {
        let url = self.base_url.join("history")?;
        let mut query: Vec<(&str, u64)> = movie_ids.iter().map(|id| ("seriesIds", *id)).collect();
        query.push(("pageSize", 100));

        let mut records = HashSet::new();
        let mut page = 1;

        loop {
            let history = self
                .client
                .get(url.clone())
                .headers(self.default_headers.clone())
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

    /// Delete series by its ID and all associated files.
    /// https://sonarr.tv/docs/api/#/Series/delete_api_v3_series__id_
    pub async fn delete_series(&self, series_id: u64) -> anyhow::Result<()> {
        let url = self
            .base_url
            .join("series/")?
            .join(&series_id.to_string())?;
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

    /// Get all tags.
    pub async fn tags(&self) -> anyhow::Result<Vec<Tag>> {
        let url = self.base_url.join("tag")?;
        let response = self
            .client
            .get(url)
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    pub title: String,
    pub id: u64,
    pub tags: Option<Vec<u64>>,
    pub statistics: SeriesStatistics,
    pub seasons: Option<Vec<Season>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesStatistics {
    pub size_on_disk: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Season {
    pub monitored: bool,
    pub statistics: SeasonStatistics,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonStatistics {
    pub next_airing: Option<String>,
    pub episode_file_count: usize,
    pub total_episode_count: usize,
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
