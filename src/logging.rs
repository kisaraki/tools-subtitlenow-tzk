//! Structured stderr diagnostics only: no file sink, telemetry, or media logging.

use std::{env, io};

use tracing::Level;

use crate::error::{AppError, Result};

pub const LOG_LEVEL_ENV: &str = "SUBTITLENOW_LOG";

pub fn init() -> Result<()> {
    let level = match env::var(LOG_LEVEL_ENV) {
        Ok(value) => parse_level(&value)?,
        Err(env::VarError::NotPresent) => Level::INFO,
        Err(env::VarError::NotUnicode(_)) => return Err(AppError::InvalidLogLevel),
    };

    tracing_subscriber::fmt()
        .json()
        .with_target(false)
        .with_max_level(level)
        .with_writer(io::stderr)
        .try_init()
        .map_err(|_| AppError::LoggingInitialization)
}

fn parse_level(value: &str) -> Result<Level> {
    match value {
        value if value.eq_ignore_ascii_case("ERROR") => Ok(Level::ERROR),
        value if value.eq_ignore_ascii_case("WARN") => Ok(Level::WARN),
        value if value.eq_ignore_ascii_case("INFO") => Ok(Level::INFO),
        value if value.eq_ignore_ascii_case("DEBUG") => Ok(Level::DEBUG),
        value if value.eq_ignore_ascii_case("TRACE") => Ok(Level::TRACE),
        _ => Err(AppError::InvalidLogLevel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_all_five_log_levels() {
        for (input, expected) in [
            ("error", Level::ERROR),
            ("WARN", Level::WARN),
            ("info", Level::INFO),
            ("DEBUG", Level::DEBUG),
            ("trace", Level::TRACE),
        ] {
            assert_eq!(parse_level(input).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_level_does_not_echo_input() {
        for input in ["", "1", "3", "5", "OFF", "private-value-not-a-level"] {
            let error = parse_level(input).unwrap_err();
            assert!(matches!(error, AppError::InvalidLogLevel));
            assert!(!error.to_string().contains("private-value"));
        }
    }
}
