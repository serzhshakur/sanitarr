use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(
    name = "Sanitarr",
    version = "1.0",
    about = "Sanitarr is a simple CLI tool to delete watched items from your *arr stack."
)]
pub struct Cli {
    /// Perform actual deletion of files. If not set the program will operate in
    /// a "dry run" mode
    #[clap(short = 'd', long)]
    pub force_delete: bool,
    /// Set the log level
    #[clap(short, long, default_value = "info", env = "LOG_LEVEL")]
    pub log_level: log::LevelFilter,
    /// Path to the config file
    #[clap(short, long)]
    pub config: PathBuf,
}
