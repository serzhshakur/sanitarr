use clap::Parser;
use cli::Cli;
use log::LevelFilter;
use services::{DownloadClient, Jellyfin, Radarr};

mod cli;
mod config;
mod http;
mod services;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    configure_log(&args.log_level)?;

    let config = config::Config::load("config.toml").await?;
    let jellyfin = Jellyfin::new(&config.username, &config.jellyfin)?;
    let items = jellyfin.get_watched_items().await?;

    let radarr = Radarr::new(&config.radarr)?;
    let download_ids = radarr.cleanup_and_get_download_ids(&items).await?;

    let download_client = DownloadClient::new(&config.download_client).await?;
    download_client.delete(true, &download_ids).await?;

    Ok(())
}

fn configure_log(level: &LevelFilter) -> anyhow::Result<()> {
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
