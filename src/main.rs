use clap::Parser;
use cleaners::{MoviesCleaner, SeriesCleaner};
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
    let download_service = DownloadService::new(config.download_clients).await?;
    let user = jellyfin_client.user(&config.username).await?;

    let movies_cleaner = MoviesCleaner::new(
        config.radarr,
        jellyfin_client.clone(),
        download_service.clone(),
        &user.id,
    )?;

    let series_cleaner = SeriesCleaner::new(
        config.sonarr,
        jellyfin_client.clone(),
        download_service.clone(),
        &user.id,
    )?;

    tokio::try_join!(
        movies_cleaner.cleanup(args.force_delete),
        series_cleaner.cleanup(args.force_delete),
    )?;

    Ok(())
}
