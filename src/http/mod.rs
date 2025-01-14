pub mod jellyfin_client;
pub mod qbittorrent_client;
pub mod radarr_client;

pub use jellyfin_client::JellyfinClient;
use log::debug;
pub use qbittorrent_client::QbittorrentClient;
pub use radarr_client::RadarrClient;

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
