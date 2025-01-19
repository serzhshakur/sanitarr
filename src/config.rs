use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub username: String,
    pub jellyfin: JellyfinConfig,
    pub radarr: RadarrConfig,
    pub sonarr: SonarrConfig,
    pub download_client: DownloadClientConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JellyfinConfig {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarrConfig {
    pub base_url: String,
    pub api_key: String,
    #[serde(with = "humantime_serde")]
    pub retention_period: Duration,
    #[serde(default)]
    pub tags_to_keep: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SonarrConfig {
    pub base_url: String,
    pub api_key: String,
    #[serde(with = "humantime_serde")]
    pub retention_period: Duration,
    #[serde(default)]
    pub tags_to_keep: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum DownloadClientConfig {
    Qbittorrent(QbittorrentConfig),
    // add more clients here
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QbittorrentConfig {
    pub username: String,
    pub password: String,
    pub base_url: String,
}

impl Config {
    pub async fn load(path: &str) -> anyhow::Result<Self> {
        let config = tokio::fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&config)?;
        Ok(config)
    }
}
