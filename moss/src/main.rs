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
/// Report an execution error to the user
fn report_error(error: &dyn Error) {
    let mut chain_parts = Vec::new();
    let mut current: &dyn Error = error;
    loop {
        chain_parts.push(current.to_string());
        if let Some(source) = current.source() {
            current = source;
        } else {
            break;
        }
    }
    let chain = chain_parts.join(": ");
    eprintln!("{}: {}", "Error".red(), chain);
    error!(%chain, "Command execution failed");
}

