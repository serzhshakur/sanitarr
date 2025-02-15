use clap::Parser;
use cleaners::{MoviesCleaner, SeriesCleaner};
use cli::Cli;
use http::JellyfinClient;
use log::LevelFilter;
use services::DownloadService;

mod cleaners;
mod cli;
mod config;
mod http;
mod services;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    setup_logging(&args.log_level)?;

    let config = config::Config::load("config.toml").await?;

    let jellyfin_client = JellyfinClient::new(&config.jellyfin)?;
    let download_client = DownloadService::new(&config.download_client).await?;

    let movies_cleaner = MoviesCleaner::new(
        config.radarr,
        jellyfin_client.clone(),
        download_client.clone(),
    )?;

    let series_cleaner = SeriesCleaner::new(
        config.sonarr,
        jellyfin_client.clone(),
        download_client.clone(),
    )?;

    movies_cleaner
        .cleanup(&config.username, args.force_delete)
        .await?;
    series_cleaner
        .cleanup(&config.username, args.force_delete)
        .await?;

    Ok(())
}

fn setup_logging(level: &LevelFilter) -> anyhow::Result<()> {
    fern::Dispatch::new()
        .level(*level)
        .format(|out, message, record| {
            out.finish(format_args!(
                "{timestamp} [{level}] {message}",
                timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                level = record.level(),
                message = message,
            ))
        })
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}
