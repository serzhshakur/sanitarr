mod jellyfin_client;
mod qbittorrent_client;
mod radarr_client;
mod sonarr_client;

pub use jellyfin_client::{Item, ItemsFilter, JellyfinClient, UserId};
use log::debug;
pub use qbittorrent_client::QbittorrentClient;
pub use radarr_client::{Movie, RadarrClient};
#[cfg(test)]
pub use sonarr_client::{Season, SeasonStatistics, SeriesStatistics};
pub use sonarr_client::{SeriesInfo, SonarrClient};

use anyhow::bail;
use reqwest::Response;

trait ResponseExt {
    async fn handle_error(self) -> anyhow::Result<Response>;
}

impl ResponseExt for Response {
    async fn handle_error(self) -> anyhow::Result<Response> {
        let url = self.url();
        if self.status().is_success() {
            debug!("request to {url} succeeded");
            Ok(self)
        } else {
            let status = self.status();
            let url = url.clone();
            let body = self.text().await?;
            bail!("request to {url} failed with status {status}: {body}")
        }
    }
}
