use clap::Parser;
use cli::Cli;
use log::{info, LevelFilter};
use services::{DownloadClient, Jellyfin, Radarr};

mod cli;
mod config;
mod http;
mod services;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    setup_logging(&args.log_level)?;

    let config = config::Config::load("config.toml").await?;
    let jellyfin = Jellyfin::new(&config.username, &config.jellyfin)?;
    let items = jellyfin.get_watched_items().await?;

    if items.is_empty() {
        info!("no items found for deletion!");
    } else {
        let radarr = Radarr::new(&config.radarr)?;
        let download_ids = radarr
            .delete_and_get_download_ids(args.force_delete, &items)
            .await?;

        let download_client = DownloadClient::new(&config.download_client).await?;
        download_client
            .delete(args.force_delete, &download_ids)
            .await?;
    }

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
