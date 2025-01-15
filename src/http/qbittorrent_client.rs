use std::collections::HashSet;

use super::ResponseExt;
use crate::config::QbittorrentConfig;
use anyhow::Ok;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::json;

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

    /// List all torrents in the client by their hashes.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#get-torrent-list
    pub async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<Torrent>> {
        let url = self.base_url.join("torrents/info")?;
        let hashes = hashes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("|");

        let response = self
            .client
            .get(url)
            .query(&[("hashes", hashes)])
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    /// Delete torrents by provided hashes and also delete the associated files.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#delete-torrents
    pub async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()> {
        let url = self.base_url.join("torrents/delete")?;
        let hashes = hashes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("|");
        let body = &[("hashes", hashes.as_str()), ("deleteFiles", "true")];
        self.client
            .post(url)
            .form(body)
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?;
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct Torrent {
    pub name: String,
}
