use log::LevelFilter;
use std::str::FromStr;

const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

/// setup logging for the application including line format as well as the main
/// log level and per-target log levels (if provided)
pub fn setup_logging(level: LoggingSettings) -> anyhow::Result<()> {
    let mut cfg = fern::Dispatch::new()
        .level(level.root_level)
        .format(|out, message, record| {
            out.finish(format_args!(
                "{timestamp} [{level}] {message}",
                timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                level = record.level(),
                message = message,
            ))
        })
        .chain(std::io::stdout());

    for (log_target, level) in level.other_levels {
        cfg = cfg.level_for(log_target, level);
    }
    cfg.apply()?;
    Ok(())
}

#[derive(Debug, Clone)]
/// Represents the logging settings for the application, including the root log
/// level and specific log levels for other modules. This allows to separately
/// configure log levels for different packages.
///
/// Examples:
///   - `info`
///   - `off,sanitarr=debug,reqwest=info`
///   - `off,sanitarr::http=info,sanitarr::services=debug`
pub struct LoggingSettings {
    pub root_level: LevelFilter,
    pub other_levels: Vec<(String, LevelFilter)>,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            root_level: DEFAULT_LEVEL,
            other_levels: Vec::new(),
        }
    }
}

impl FromStr for LoggingSettings {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(',');
        let root_level = parts.next().unwrap_or("info");
        let root_level = LevelFilter::from_str(root_level).unwrap_or(DEFAULT_LEVEL);
        let other_levels = parts
            .filter_map(|s| {
                let mut subparts = s.split('=');
                let (Some(log_target), Some(level)) = (subparts.next(), subparts.next()) else {
                    return None;
                };
                let level = LevelFilter::from_str(level).unwrap_or(DEFAULT_LEVEL);
                Some((log_target.to_string(), level))
            })
            .collect::<Vec<_>>();

        Ok(Self {
            root_level,
            other_levels,
        })
    }
}

#[cfg(test)]
mod test_super {
    use super::*;

    #[test]
    fn test_deser_single_log_level() {
        let raw_str = "debug";
        let settings = LoggingSettings::from_str(raw_str).unwrap();
        assert_eq!(settings.root_level, LevelFilter::Debug);
        assert!(settings.other_levels.is_empty());
    }

    #[test]
    fn test_deser_extra_log_levels() {
        let raw_str = "off,sanitarr=debug,reqwest=info";
        let settings = LoggingSettings::from_str(raw_str).unwrap();

        assert_eq!(settings.root_level, LevelFilter::Off);
        assert_eq!(settings.other_levels.len(), 2);

        let (log_target_one, level_one) = &settings.other_levels[0];
        assert_eq!(log_target_one, "sanitarr");
        assert_eq!(level_one, &LevelFilter::Debug);

        let (log_target_two, level_two) = &settings.other_levels[1];
        assert_eq!(log_target_two, "reqwest");
        assert_eq!(level_two, &LevelFilter::Info);
    }
}
