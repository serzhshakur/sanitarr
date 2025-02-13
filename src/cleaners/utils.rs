use chrono::{DateTime, Utc};

/// a helper function that turns the difference between `last_played_dt` and
/// `retention_dt` into a human readable string
pub fn retention_str(last_played_dt: &DateTime<Utc>, retention_dt: &DateTime<Utc>) -> String {
    if retention_dt > last_played_dt {
        "0".to_string()
    } else {
        let delta = *last_played_dt - retention_dt;
        let days = delta.num_days();
        if days > 0 {
            format!("{days} day{}", suffix(days))
        } else {
            let hours = delta.num_hours();
            if hours > 0 {
                format!("{hours} hour{}", suffix(hours))
            } else {
                let minutes = delta.num_minutes();
                format!("{minutes} minute{}", suffix(minutes))
            }
        }
    }
}

fn suffix(units: i64) -> String {
    (units > 1).then_some("s").unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_retention_str_less_than_zero() {
        let retention = chrono::Utc::now();
        let last_played = chrono::Utc::now() - Duration::from_secs(3600);
        assert_eq!(retention_str(&last_played, &retention), "0");
    }

    #[test]
    fn test_retention_str_one_day() {
        let retention = chrono::Utc::now() - Duration::from_secs(60 * 60 * 24 + 300);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "1 day");
    }

    #[test]
    fn test_retention_str_days() {
        let retention = chrono::Utc::now() - Duration::from_secs(60 * 60 * 24 * 3 + 300);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "3 days");
    }

    #[test]
    fn test_retention_str_one_hour() {
        let retention = chrono::Utc::now() - Duration::from_secs(60 * 60);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "1 hour");
    }

    #[test]
    fn test_retention_str_hours() {
        let retention = chrono::Utc::now() - Duration::from_secs(60 * 60 * 13);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "13 hours");
    }

    #[test]
    fn test_retention_str_one_minute() {
        let retention = chrono::Utc::now() - Duration::from_secs(70);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "1 minute");
    }

    #[test]
    fn test_retention_str_minutes() {
        let retention = chrono::Utc::now() - Duration::from_secs(125);
        let last_played = chrono::Utc::now();
        assert_eq!(retention_str(&last_played, &retention), "2 minutes");
    }
}
