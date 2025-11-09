// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::error::Error;

use tracing::error;
use tui::Styled;

mod cli;

/// Main entry point
fn main() {
    if let Err(err) = cli::process() {
        report_error(&err);
        std::process::exit(1);
    }
}

/// Display a formatted error message and log the details
fn report_error(err: &cli::Error) {
    let chain = collect_sources(err);
    let message = chain.join(": ");
    error!(?message, "Command execution failed");
    println!("{}: {message}", "Error".red());
}

/// Recursively gather error sources into a string list
fn collect_sources(err: &cli::Error) -> Vec<String> {
    let mut result = vec![err.to_string()];
    let mut source = err.source();
    while let Some(next) = source {
        result.push(next.to_string());
        source = next.source();
    }
    result
}
