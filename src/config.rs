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
    #[serde(with = "humantime_serde", default)]
    pub retention_period: Option<Duration>,
    #[serde(default)]
    pub tags_to_keep: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SonarrConfig {
    pub base_url: String,
    pub api_key: String,
    #[serde(with = "humantime_serde", default)]
    pub retention_period: Option<Duration>,
    #[serde(default)]
    pub tags_to_keep: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum DownloadClientConfig {
    #[serde(alias = "qbittorrent")]
    Qbittorrent(QbittorrentConfig),
    #[serde(alias = "deluge")]
    Deluge(DelugeConfig),
    // add more clients here
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QbittorrentConfig {
    pub username: String,
    pub password: String,
    pub base_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelugeConfig {
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

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_parse_config() -> anyhow::Result<()> {
        let cfg = Config::load("example.config.toml").await?;
        assert_eq!(cfg.username, "foo");

        assert_eq!(cfg.jellyfin.api_key, "api-key-foo");
        assert_eq!(cfg.jellyfin.base_url, "http://localhost:8096");

        assert_eq!(cfg.radarr.base_url, "http://localhost:7878");
        assert_eq!(cfg.radarr.api_key, "api-key-foo");
        assert_eq!(&cfg.radarr.tags_to_keep, &["keep".to_owned()]);
        let dur = 60 * 60 * 24 * 2;
        assert_eq!(cfg.radarr.retention_period, Some(Duration::from_secs(dur)));

        assert_eq!(cfg.sonarr.base_url, "http://localhost:7878");
        assert_eq!(cfg.sonarr.api_key, "api-key-foo");
        assert_eq!(&cfg.sonarr.tags_to_keep, &["keep".to_owned()]);
        let dur = 60 * 60 * 24 * 7;
        assert_eq!(cfg.sonarr.retention_period, Some(Duration::from_secs(dur)));

        let DownloadClientConfig::Qbittorrent(cfg) = cfg.download_client else {
            panic!("wrong download client");
        };
        assert_eq!(cfg.base_url, "http://localhost:8080");
        assert_eq!(cfg.username, "admin");
        assert_eq!(cfg.password, "adminadmin");

        Ok(())
    }
}
