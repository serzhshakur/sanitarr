use super::TorrentClient;
use crate::config::QbittorrentConfig;
use crate::http::{ResponseExt, TorrentInfo};
use anyhow::Ok;
use async_trait::async_trait;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

const WATCHED_TAG: &str = "watched";

pub struct QbittorrentClient {
    client: Client,
    base_url: Url,
    default_headers: HeaderMap,
}

impl QbittorrentClient {
    pub async fn new(config: &QbittorrentConfig) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(&config.base_url)?;
        base_url.set_path("/api/v2/");

        let client = Client::new();

        let response = client
            .post(base_url.join("auth/login")?)
            .form(&json!({ "username": config.username, "password": config.password }))
            .send()
            .await?
            .handle_error()
            .await?;

        let sid_cookie = response
            .cookies()
            .find(|c| c.name().to_lowercase().trim() == "sid")
            .map(|c| c.value().to_owned())
            .unwrap_or_default();

        let mut default_headers = HeaderMap::new();
        let mut header_value = HeaderValue::from_str(&format!("SID={sid_cookie}"))?;
        header_value.set_sensitive(true);
        default_headers.insert(COOKIE, header_value);

        Ok(Self {
            client,
            base_url,
            default_headers,
        })
    }

    async fn list(&self, field_name: &str, field_value: &str) -> anyhow::Result<Vec<TorrentInfo>> {
        let url = self.base_url.join("torrents/info")?;
        let torrents: Vec<Torrent> = self
            .client
            .get(url)
            .query(&[(field_name, field_value)])
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;

        Ok(torrents.into_iter().map(From::from).collect())
    }
}

#[async_trait]
impl TorrentClient for QbittorrentClient {
    fn is_delayed_deletion_supported(&self) -> bool {
        true
    }

    /// List all torrents in the client by their hashes.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#get-torrent-list
    async fn list(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
        let hashes = to_bar_separated_string(hashes);
        let torrents = self.list("hashes", &hashes).await?;
        Ok(torrents)
    }

    /// List all torrents in the client by "watched" tag.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#get-torrent-list
    async fn list_watched(&self) -> anyhow::Result<Vec<TorrentInfo>> {
        let torrents = self.list("tag", WATCHED_TAG).await?;
        Ok(torrents)
    }

    /// Delete torrents by provided hashes and also delete the associated files.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#delete-torrents
    async fn delete(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
        let url = self.base_url.join("torrents/delete")?;
        let hashes = to_bar_separated_string(hashes);
        let body = &[("hashes", hashes.as_str()), ("deleteFiles", "true")];
        let torrents: Vec<Torrent> = self
            .client
            .post(url)
            .form(body)
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(torrents.into_iter().map(From::from).collect())
    }

    async fn mark_as_watched(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<TorrentInfo>> {
        let url = self.base_url.join("torrents/addTags")?;
        let hashes = to_bar_separated_string(hashes);
        let body = &[("hashes", hashes.as_str()), ("tags", WATCHED_TAG)];
        let torrents: Vec<Torrent> = self
            .client
            .post(url)
            .form(body)
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(torrents.into_iter().map(From::from).collect())
    }
}

fn to_bar_separated_string<'a, I>(hashes: I) -> String
where
    I: IntoIterator<Item = &'a String>,
{
    let hashes_vec = hashes.into_iter().map(String::as_str).collect::<Vec<_>>();
    if hashes_vec.is_empty() {
        // if there are no hashes, return "none" as the value to avoid
        // qbittorrent returning all torrents
        return "none".to_owned();
    }
    hashes_vec.join("|")
}

#[derive(Deserialize)]
struct Torrent {
    hash: String,
    name: String,
    ratio: f64,
    seeding_time: u64,
    tracker: String,
}

impl From<Torrent> for TorrentInfo {
    fn from(
        Torrent {
            hash,
            name,
            ratio,
            seeding_time,
            tracker,
        }: Torrent,
    ) -> Self {
        let seed_time = Duration::from_secs(seeding_time);
        Self {
            name,
            hash,
            ratio,
            seed_time,
            tracker,
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_to_bar_separated_string() {
        let hashes = &["hash1".to_owned(), "hash2".to_owned(), "hash3".to_owned()];
        let result = super::to_bar_separated_string(hashes);
        assert_eq!(result, "hash1|hash2|hash3");
    }

    #[test]
    fn test_to_bar_separated_string_empty() {
        let hashes: Vec<String> = vec![];
        let result = super::to_bar_separated_string(&hashes);
        assert_eq!(result, "none");
    }
}
