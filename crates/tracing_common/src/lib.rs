// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{fs::OpenOptions, io, str::FromStr};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub enum OutputDestination {
    Stderr,
    File(String),
}

/// Initialize tracing subscriber with the specified format, log level, and destination
pub fn init(format: OutputFormat, level: LevelFilter, destination: OutputDestination) {
    let filter = tracing_subscriber::filter::Targets::new()
        .with_default(level)
        // these log a lot of stuff when downloading.
        // it's very rare to need to debug HTTP issues, and then it might often be more
        // helpful to set up tcpdump or wireshark anyways.
        .with_target("h2", LevelFilter::INFO)
        .with_target("hyper", LevelFilter::INFO)
        .with_target("hyper_util", LevelFilter::INFO);

    match (format, destination) {
        (OutputFormat::Text, OutputDestination::Stderr) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(io::stderr))
                .init();
        }
        (OutputFormat::Json, OutputDestination::Stderr) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_writer(io::stderr))
                .init();
        }
        (OutputFormat::Text, OutputDestination::File(path)) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("Failed to open log file {}: {}", path, e));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(file))
                .init();
        }
        (OutputFormat::Json, OutputDestination::File(path)) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("Failed to open log file {}: {}", path, e));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_writer(file))
                .init();
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub level: LevelFilter,
    pub format: OutputFormat,
    pub destination: OutputDestination,
}

impl FromStr for LogConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();

        if parts.is_empty() || parts.len() > 3 {
            return Err("Invalid log format. Expected: <level>[:<format>][:<destination>]".to_string());
        }

        let level = match parts[0].to_lowercase().as_str() {
            "trace" => LevelFilter::TRACE,
            "debug" => LevelFilter::DEBUG,
            "info" => LevelFilter::INFO,
            "warn" => LevelFilter::WARN,
            "error" => LevelFilter::ERROR,
            _ => {
                return Err(format!(
                    "Invalid log level: {}. Valid levels: trace, debug, info, warn, error",
                    parts[0]
                ));
            }
        };

        let format = if parts.len() >= 2 {
            match parts[1].to_lowercase().as_str() {
                "text" => OutputFormat::Text,
                "json" => OutputFormat::Json,
                _ => return Err(format!("Invalid log format: {}. Valid formats: text, json", parts[1])),
            }
        } else {
            OutputFormat::Text
        };

        let destination = if parts.len() == 3 {
            if parts[2] == "stderr" {
                OutputDestination::Stderr
            } else {
                OutputDestination::File(parts[2].to_string())
            }
        } else {
            OutputDestination::Stderr
        };

        Ok(LogConfig {
            level,
            format,
            destination,
        })
    }
}

/// Initialize tracing with a parsed log configuration
pub fn init_with_config(config: LogConfig) {
    init(config.format, config.level, config.destination);
}
