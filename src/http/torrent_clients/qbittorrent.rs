use super::TorrentClient;
use crate::config::QbittorrentConfig;
use crate::http::ResponseExt;
use anyhow::Ok;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

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
}

#[async_trait]
impl TorrentClient for QbittorrentClient {
    /// List all torrents in the client by their hashes.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#get-torrent-list
    async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<String>> {
        let url = self.base_url.join("torrents/info")?;
        let hashes = to_bar_separated_string(hashes);
        let response: Vec<Torrent> = self
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

        Ok(response.into_iter().map(|t| t.name).collect())
    }

    /// Delete torrents by provided hashes and also delete the associated files.
    /// https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)#delete-torrents
    async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()> {
        let url = self.base_url.join("torrents/delete")?;
        let hashes = to_bar_separated_string(hashes);
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
pub struct Torrent {
    pub name: String,
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
