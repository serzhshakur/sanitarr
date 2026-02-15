use super::{ResponseExt, TorrentClientKind};
use anyhow::Ok;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, ClientBuilder, Url};
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt::Debug;

/// A client for interacting with Sonarr API.
/// https://sonarr.tv/docs/api/#v3
pub struct SonarrClient {
    client: Client,
    base_url: Url,
}

impl SonarrClient {
    pub fn new(base_url: &str, api_key: &str) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(base_url)?;
        base_url.set_path("/api/v3/");

        let default_headers = auth_headers(api_key)?;
        let client = ClientBuilder::new()
            .default_headers(default_headers)
            .build()?;

        Ok(Self { client, base_url })
    }

    /// Get the series IDs for a given TVDB ID.
    /// https://sonarr.tv/docs/api/#v3/tag/series/GET/api/v3/series
    pub async fn series_by_tvdb_id(&self, provider_id: &str) -> anyhow::Result<Vec<SeriesInfo>> {
        let url = self.base_url.join("series")?;
        let response = self
            .client
            .get(url)
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
    /// https://sonarr.tv/docs/api/#v3/tag/history/GET/api/v3/history
    pub async fn history_records(
        &self,
        movie_ids: &HashSet<u64>,
    ) -> anyhow::Result<HashSet<HistoryRecord>> {
        let url = self.base_url.join("history")?;
        let mut query: Vec<_> = movie_ids.iter().map(|id| ("seriesIds", *id)).collect();
        // event type 1 = "grabbed", see docs for more info:
        // https://github.com/Sonarr/Sonarr/blob/v5-develop/src/NzbDrone.Core/History/EpisodeHistory.cs#L37
        query.push(("eventType", 1));
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

    /// Delete series by its ID and all associated files.
    /// https://sonarr.tv/docs/api/#v3/tag/series/DELETE/api/v3/series/{id}
    pub async fn delete_series(&self, series_id: u64) -> anyhow::Result<()> {
        let url = self
            .base_url
            .join("series/")?
            .join(&series_id.to_string())?;
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

    /// Get episodes for a given series ID
    /// https://sonarr.tv/docs/api/#v3/tag/episode/GET/api/v3/episode
    pub async fn episodes_by_series_id(&self, series_id: u64) -> anyhow::Result<Vec<Episode>> {
        let url = self.base_url.join("episode")?;
        let response = self
            .client
            .get(url)
            .query(&[("seriesId", series_id)])
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    /// Unmonitor episodes through the episodes monitor API
    /// https://sonarr.tv/docs/api/#v3/tag/episode/PUT/api/v3/episode/monitor
    pub async fn unmonitor_episodes(
        &self,
        episode_ids: &HashSet<u64>,
    ) -> anyhow::Result<Vec<EpisodeMonitorResponse>> {
        let url = self.base_url.join("episode/monitor")?;
        let request = serde_json::json!({
            "episodeIds": episode_ids,
            "monitored": false,
        });
        let res = self
            .client
            .put(url)
            .json(&request)
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(res)
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
#[cfg_attr(test, derive(Default))]
pub struct SeriesInfo {
    pub title: String,
    pub id: u64,
    pub tags: Option<Vec<u64>>,
    pub statistics: SeriesStatistics,
    pub seasons: Option<Vec<Season>>,
}

impl Debug for SeriesInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.title, self.id)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(Default))]
pub struct SeriesStatistics {
    pub size_on_disk: usize,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub episode_number: u32,
    pub id: u64,
    pub monitored: bool,
    pub season_number: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Season {
    pub statistics: SeasonStatistics,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonStatistics {
    pub next_airing: Option<DateTime<Utc>>,
    pub episode_file_count: usize,
    pub total_episode_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub records: HashSet<HistoryRecord>,
}

#[derive(Deserialize, Hash, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub download_id: Option<String>,
    pub data: Option<HistoryRecordData>,
}

impl HistoryRecord {
    pub fn download_id_per_client(self) -> Option<(TorrentClientKind, String)> {
        let download_id = self.download_id?;
        let client = self.data?.download_client?;
        Some((client, download_id))
    }
}

#[derive(Deserialize, Hash, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecordData {
    pub download_client: Option<TorrentClientKind>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub label: String,
    pub id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeMonitorResponse {
    season_number: u16,
    episode_number: u16,
    pub id: u64,
}

impl std::fmt::Display for EpisodeMonitorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s{:02}e{:02}", self.season_number, self.episode_number)
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryRecord;
    use crate::http::sonarr_client::HistoryRecordData;

    #[test]
    fn test_auth_headers() {
        let headers = super::auth_headers("abc-key").unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("x-api-key").unwrap(), "abc-key");
    }

    #[test]
    fn test_download_id_and_client() {
        let history_record = HistoryRecord {
            download_id: "foo".to_owned().into(),
            data: Some(HistoryRecordData {
                download_client: Some(crate::http::TorrentClientKind::Deluge),
            }),
        };
        let (client, download_id) = history_record.download_id_per_client().unwrap();
        assert!(matches!(client, crate::http::TorrentClientKind::Deluge));
        assert_eq!(download_id, "foo");
    }

    #[test]
    fn test_download_id_and_client_no_id() {
        let history_record = HistoryRecord {
            download_id: None,
            data: Some(HistoryRecordData {
                download_client: Some(crate::http::TorrentClientKind::Deluge),
            }),
        };
        assert!(history_record.download_id_per_client().is_none());
    }

    #[test]
    fn test_download_id_and_client_no_data() {
        let history_record = HistoryRecord {
            download_id: "foo".to_owned().into(),
            data: None,
        };
        assert!(history_record.download_id_per_client().is_none());
    }

    #[test]
    fn test_download_id_and_client_no_client() {
        let history_record = HistoryRecord {
            download_id: "foo".to_owned().into(),
            data: Some(HistoryRecordData {
                download_client: None,
            }),
        };
        assert!(history_record.download_id_per_client().is_none());
    }
}
