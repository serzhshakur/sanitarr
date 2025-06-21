use anyhow::bail;
use serde::Deserialize;
use std::{path::Path, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub username: String,
    pub jellyfin: JellyfinConfig,
    pub radarr: RadarrConfig,
    pub sonarr: SonarrConfig,
    pub download_clients: DownloadClientsConfig,
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
    #[serde(default)]
    pub unmonitor: bool,
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
#[serde(deny_unknown_fields)]
pub struct DownloadClientsConfig {
    pub qbittorrent: Option<QbittorrentConfig>,
    pub deluge: Option<DelugeConfig>,
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
    pub async fn load(path: &Path) -> anyhow::Result<Self> {
        let Ok(config_str) = tokio::fs::read_to_string(path).await else {
            bail!("failed to read config file at {path:?}");
        };
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::*;
    use anyhow::Context;

    #[tokio::test]
    async fn test_parse_config() -> anyhow::Result<()> {
        let cfg = Config::load(&PathBuf::from("example.config.toml")).await?;
        assert_eq!(cfg.username, "foo");

        assert_eq!(cfg.jellyfin.api_key, "api-key-foo");
        assert_eq!(cfg.jellyfin.base_url, "http://localhost:8096");

        assert_eq!(cfg.radarr.base_url, "http://localhost:7878");
        assert_eq!(cfg.radarr.api_key, "api-key-foo");
        assert_eq!(&cfg.radarr.tags_to_keep, &["keep".to_owned()]);
        let dur = 60 * 60 * 24 * 2;
        assert_eq!(cfg.radarr.retention_period, Some(Duration::from_secs(dur)));
        assert_eq!(cfg.radarr.unmonitor, false);

        assert_eq!(cfg.sonarr.base_url, "http://localhost:7878");
        assert_eq!(cfg.sonarr.api_key, "api-key-foo");
        assert_eq!(&cfg.sonarr.tags_to_keep, &["keep".to_owned()]);
        let dur = 60 * 60 * 24 * 7;
        assert_eq!(cfg.sonarr.retention_period, Some(Duration::from_secs(dur)));

        let deluge_cfg = &cfg
            .download_clients
            .deluge
            .context("no Deluge config defined")?;

        let qbittorrent_cfg = &cfg
            .download_clients
            .qbittorrent
            .context("no qBittorrent config defined")?;

        assert_eq!(qbittorrent_cfg.base_url, "http://localhost:8080");
        assert_eq!(qbittorrent_cfg.username, "admin");
        assert_eq!(qbittorrent_cfg.password, "adminadmin");

        assert_eq!(deluge_cfg.base_url, "http://localhost:8112");
        assert_eq!(deluge_cfg.password, "qwerty");

        Ok(())
    }
}
