// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use colored::Colorize;
use cli::{self, Error};

/// Main entry point
fn main() {
    if let Err(err) = cli::process() {
        report_error(err);
        std::process::exit(1);
    }
}

fn report_error(error: Error) {
    // Collect the error chain into a single string
    let chain = std::iter::successors(Some(&error as &dyn std::error::Error), |e| e.source())
    .map(|e| e.to_string())
    .collect::<Vec<_>>()
    .join(": ");

    eprintln!("{}: {}", "Error".red(), chain);
}
