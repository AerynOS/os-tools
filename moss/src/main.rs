// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::error::Error;

use tui::Styled;

mod cli;

/// Main entry point
fn main() {
    if let Err(error) = cli::process() {
        report_error(error);
        std::process::exit(1);
    }
}

/// Report an execution error to the user
fn report_error(error: cli::Error) {
    let sources = sources(&error);
    let error = sources.join(": ");
    eprintln!("{}: {error}", "Error".red());
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
