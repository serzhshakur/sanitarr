use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub username: String,
    pub jellyfin: JellyfinConfig,
    pub radarr: RadarrConfig,
    pub sonarr: SonarrConfig,
    pub download_client: DownloadClientConfig,
}

#[derive(Deserialize)]
pub struct JellyfinConfig {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Deserialize)]
pub struct RadarrConfig {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Deserialize)]
pub struct SonarrConfig {
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub tags_to_keep: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum DownloadClientConfig {
    Qbittorrent(QbittorrentConfig),
    // add more clients here
}

#[derive(Deserialize)]
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
