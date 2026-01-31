mod jellyfin_client;
mod radarr_client;
mod sonarr_client;
mod torrent_clients;

pub use jellyfin_client::{ItemsFilter, JellyfinClient, JellyfinItem, UserId};
use log::trace;
pub use radarr_client::{Movie, MovieEditor, RadarrClient};
#[cfg(test)]
pub use sonarr_client::{Season, SeasonStatistics, SeriesStatistics};
pub use sonarr_client::{SeriesInfo, SonarrClient};
pub use torrent_clients::{DelugeClient, QbittorrentClient, TorrentClient, TorrentClientKind};

use anyhow::bail;
use reqwest::Response;

trait ResponseExt {
    async fn handle_error(self) -> anyhow::Result<Response>;
}

impl ResponseExt for Response {
    async fn handle_error(self) -> anyhow::Result<Response> {
        let url = self.url();
        if self.status().is_success() {
            trace!("request to {url} succeeded");
            Ok(self)
        } else {
            let status = self.status();
            let url = url.clone();
            let body = self.text().await?;
            bail!("request to {url} failed with status {status}: {body}")
        }
    }
}
