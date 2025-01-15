use clap::Parser;
use cli::Cli;
use log::LevelFilter;
use services::{DownloadClient, Jellyfin, Radarr, Sonarr};

mod cli;
mod config;
mod http;
mod movies_cleaner;
mod series_cleaner;
mod services;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    setup_logging(&args.log_level)?;

    let config = config::Config::load("config.toml").await?;
    let jellyfin = Jellyfin::new(&config.username, &config.jellyfin)?;
    let download_client = DownloadClient::new(&config.download_client).await?;

    let movies_cleaner = movies_cleaner::MoviesCleaner::new(
        Radarr::new(&config.radarr)?,
        jellyfin.clone(),
        download_client.clone(),
    )?;

    let series_cleaner = series_cleaner::SeriesCleaner::new(
        Sonarr::new(&config.sonarr)?,
        jellyfin.clone(),
        download_client.clone(),
    )?;

    movies_cleaner.cleanup(args.force_delete).await?;
    series_cleaner.cleanup(args.force_delete).await?;

    Ok(())
}

fn setup_logging(level: &LevelFilter) -> anyhow::Result<()> {
    let base_config = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{timestamp} [{level}] {message}",
                timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                level = record.level(),
                message = message,
            ))
        })
        .level(*level)
        .chain(std::io::stdout());
    base_config.apply()?;
    Ok(())
}
