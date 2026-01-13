use clap::Parser;
use cleaners::{EpisodesCleaner, MoviesCleaner};
use cli::Cli;
use http::JellyfinClient;
use services::DownloadService;

mod cleaners;
mod cli;
mod config;
mod http;
mod logging;
mod services;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    logging::setup_logging(args.log_level)?;

    let config = config::Config::load(&args.config).await?;

    let jellyfin_client = JellyfinClient::new(&config.jellyfin)?;
    let download_client = DownloadService::new(config.download_clients).await?;

    let movies_cleaner = MoviesCleaner::new(
        config.radarr,
        jellyfin_client.clone(),
        download_client.clone(),
    )?;

    let episodes_cleaner = EpisodesCleaner::new(
        config.sonarr,
        jellyfin_client.clone(),
        download_client.clone(),
    )?;

    movies_cleaner
        .cleanup(&config.username, args.force_delete)
        .await?;
    episodes_cleaner
        .cleanup(&config.username, args.force_delete)
        .await?;

    Ok(())
}
