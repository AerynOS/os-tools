// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::error::Error;

use tracing::Level;
use tracing_subscriber::{
    fmt,
    Layer,
    filter::LevelFilter,
    filter::filter_fn,
    Registry,
    layer::SubscriberExt,
};

mod cli;

/// Main entry point
fn main() {
    // Write log to /tmp/moss.json
    let file_appender = tracing_appender::rolling::daily("/tmp", "moss.json");
    let (non_blocking_appender, _guard) = tracing_appender::non_blocking(file_appender);

    let fmt_layer_file = fmt::layer()
        .json()
        .with_writer(non_blocking_appender);

    // Write ERROR and WARN to console
    let fmt_layer_err = fmt::layer()
        .with_level(true)
        .with_target(false)
        .without_time()
        .with_filter(LevelFilter::WARN);

    // Write INFO to console
    let fmt_layer_out = fmt::layer()
        .with_level(false)
        .with_target(false)
        .without_time()
        .with_filter(filter_fn(|metadata| {metadata.level() == &Level::INFO}));

    //
    let subscriber = Registry::default()
        .with(fmt_layer_file)
        .with(fmt_layer_err)
        .with(fmt_layer_out);

    tracing::subscriber::set_global_default(subscriber)
        .expect("Unable to set a global subscriber");

    if let Err(error) = cli::process() {
        report_error(error);
        std::process::exit(1);
    }
}

/// Report an execution error to the user
fn report_error(error: cli::Error) {
    let sources = sources(&error);
    let error = sources.join(": ");
    tracing::error!("{error}");
}

/// Accumulate sources through error chains
fn sources(error: &cli::Error) -> Vec<String> {
    let mut sources = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source.take() {
        sources.push(error.to_string());
        source = error.source();
    }
    sources
}
