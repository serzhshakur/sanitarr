use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub username: String,
    pub jellyfin: JellyfinConfig,
    pub radarr: RadarrConfig,
    pub sonarr: SonarrConfig,
    pub download_clients: Vec<DownloadClientConfig>,
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
        let config_str = tokio::fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&config_str)?;
        
        Ok(config)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Context;

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

        let deluge_cfg = &cfg
            .download_clients
            .iter()
            .find(|cfg| matches!(cfg, DownloadClientConfig::Deluge(_)))
            .context("unable to get Deluge config")?;

        let qbittorrent_cfg = &cfg
            .download_clients
            .iter()
            .find(|cfg| matches!(cfg, DownloadClientConfig::Qbittorrent(_)))
            .context("unable to get qBittorrent config")?;

        let DownloadClientConfig::Qbittorrent(qbittorrent_cfg) = qbittorrent_cfg else {
            panic!("not a Qbittorrent client config");
        };

        assert_eq!(qbittorrent_cfg.base_url, "http://localhost:8080");
        assert_eq!(qbittorrent_cfg.username, "admin");
        assert_eq!(qbittorrent_cfg.password, "adminadmin");

        let DownloadClientConfig::Deluge(deluge_cfg) = deluge_cfg else {
            panic!("not a Deluge client config");
        };

        assert_eq!(deluge_cfg.base_url, "http://localhost:8112");
        assert_eq!(deluge_cfg.password, "qwerty");

        Ok(())
    }
}
