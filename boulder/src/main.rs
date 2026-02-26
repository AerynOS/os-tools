// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

mod architecture;
mod build;
mod cli;
mod container;
mod draft;
mod env;
mod macros;
mod package;
mod paths;
mod profile;
mod recipe;
mod timing;
mod upstream;

pub use architecture::Architecture;
pub use env::Env;
pub use macros::Macros;
pub use paths::Paths;
pub use profile::Profile;
pub use recipe::Recipe;
pub use timing::Timing;

use std::error::Error;

use tui::Styled;
fn main() {
    if let Err(error) = cli::process() {
        report_error(error);
        std::process::exit(1);
    }
}

fn report_error(error: cli::Error) {
    let sources = sources(&error);
    let error = sources.join(": ");
    eprintln!("{}: {error}", "Error".red());
}

fn sources(error: &cli::Error) -> Vec<String> {
    let mut sources = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source.take() {
        sources.push(error.to_string());
        source = error.source();
    }
    sources
}
