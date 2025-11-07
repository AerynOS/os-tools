// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

mod cli;
use crate::cli::Error;

use tracing::error;

/// Main entry point
fn main() {
    if let Err(err) = cli::process() {
        report_error(err);
        std::process::exit(1);
    }
}

/// Report an execution error to the user
fn report_error(error: Error) {
    // Collect the full error chain into a single string
    let chain = std::iter::successors(Some(&error as &dyn std::error::Error), |e| e.source())
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join(": ");

    // Log with tracing and print to console
    error!(%chain, "Command execution failed");
    println!("Error: {chain}");
}
