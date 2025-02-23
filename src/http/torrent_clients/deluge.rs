use super::TorrentClient;
use crate::config::DelugeConfig;
use crate::http::ResponseExt;
use anyhow::{bail, Context, Ok};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE};
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const SESSION_COOKIE: &str = "_session_id";

pub struct DelugeClient {
    client: Client,
    base_url: Url,
    default_headers: HeaderMap,
}

impl DelugeClient {
    pub async fn new(config: &DelugeConfig) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(&config.base_url)?;
        base_url.set_path("/json");

        let client = Client::new();
        let session_cookie = login(&client, &base_url, &config.password).await?;

        let mut default_headers = HeaderMap::new();
        let mut header_value =
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={session_cookie}"))?;
        header_value.set_sensitive(true);
        default_headers.insert(COOKIE, header_value);

        Ok(Self {
            client,
            base_url,
            default_headers,
        })
    }

    /// Internal function for submitting requests to Deluge API
    async fn post<T: DeserializeOwned>(
        &self,
        request: DelugeRequest<'_>,
    ) -> anyhow::Result<Option<T>> {
        let response = self
            .client
            .post(self.base_url.clone())
            .json(&request.to_json())
            .headers(self.default_headers.clone())
            .send()
            .await?
            .handle_error()
            .await?
            .json::<DelugeResponse>()
            .await?;

        response.response()
    }
}

#[async_trait]
impl TorrentClient for DelugeClient {
    /// List all torrents in the client by their hashes.
    async fn list_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<Vec<String>> {
        let request = DelugeRequest::ListTorrents(hashes);
        let response = self.post::<HashMap<String, Torrent>>(request).await?;

        let Some(result) = response else {
            return Ok(Vec::default());
        };

        Ok(result.into_values().map(|v| v.name).collect())
    }

    /// Delete torrents by provided hashes and also delete the associated files.
    async fn delete_torrents(&self, hashes: &HashSet<String>) -> anyhow::Result<()> {
        let request = DelugeRequest::DeleteTorrents(hashes);
        self.post::<Vec<bool>>(request).await?;

        Ok(())
    }
}

/// Login to Deluge api with password-only method
async fn login(client: &Client, url: &Url, password: &str) -> Result<String, anyhow::Error> {
    let request = DelugeRequest::Login(password);
    let response = client
        .post(url.clone())
        .json(&request.to_json())
        .send()
        .await?
        .handle_error()
        .await?;

    let sid_cookie = response
        .cookies()
        .find(|c| c.name().to_lowercase().trim() == SESSION_COOKIE)
        .map(|c| c.value().to_owned())
        .with_context(|| {
            format!("unable to get {SESSION_COOKIE} cookie from Deluge login response")
        });

    response
        .json::<DelugeResponse>()
        .await?
        .response::<bool>()?;

    sid_cookie
}

// Requests //

enum DelugeRequest<'a> {
    Login(&'a str),
    ListTorrents(&'a HashSet<String>),
    DeleteTorrents(&'a HashSet<String>),
}

impl DelugeRequest<'_> {
    fn to_json(&self) -> Value {
        match self {
            DelugeRequest::Login(password) => json!(
                {
                    "method": "auth.login",
                    "params": [password],
                    "id": 1
                }
            ),
            DelugeRequest::ListTorrents(hashes) => json!(
                {
                    "method": "core.get_torrents_status",
                    "params": [
                        { // filter
                            "id": hashes_to_lower(hashes),
                            "state": ["Seeding"]
                        },
                        // fields to return
                        ["name", "state"]
                    ],
                    "id": 1
                }
            ),
            DelugeRequest::DeleteTorrents(hashes) => json!(
                {
                    "method": "core.remove_torrents",
                    "params": [
                        // torrent hashes to delete
                        hashes_to_lower(hashes),
                        // whether to also delete torrent files
                        true
                    ],
                    "id": 1
                }
            ),
        }
    }
}

/// Both Radarr and Sonarr store download ids (torrent hashes) in uppercase
/// format. However Deluge only supports lowercased hash strings. Hence the
/// transformation
fn hashes_to_lower(hashes: &HashSet<String>) -> HashSet<String> {
    hashes.iter().map(|h| h.to_lowercase()).collect()
}

// Responses //

#[derive(Deserialize)]
pub struct Torrent {
    pub name: String,
}

#[derive(Deserialize)]
struct DelugeError {
    message: String,
    code: i64,
}

#[derive(Deserialize)]
struct DelugeResponse {
    result: Option<Value>,
    error: Option<DelugeError>,
}

impl DelugeResponse {
    /// processes response received from Deluge API. Throws an error if response
    /// contains a non-null `error` or it's `result` is set to `false`.
    /// Otherwise deserializes the `result` into a type `T` and returns the
    /// deserialized value
    fn response<T: DeserializeOwned>(self) -> anyhow::Result<Option<T>> {
        if let Some(DelugeError { message, code }) = self.error {
            bail!("failed to call Deluge api: {message} (error code {code})")
        }

        if let Some(result) = self.result {
            if let Value::Bool(false) = result {
                bail!("Deluge API returned falsy response")
            }
            return Ok(serde_json::from_value(result)?);
        }
        Ok(None)
    }
}
