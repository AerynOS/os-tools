// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use moss::{Installation, client::Client, environment};
use tracing::instrument;

pub use moss::client::Error;

pub fn command() -> clap::Command {
    Command::command()
}

#[derive(Debug, Parser)]
#[command(
    name = "fetch",
    visible_alias = "fe",
    about = "Fetch package(s)",
    long_about = "Fetch package stone(s) by name"
)]
struct Command {
    /// directory to write the fetched stone(s)
    #[arg(short, long, default_value = ".")]
    output_dir: PathBuf,
    /// packages to fetch
    #[arg(name = "PACKAGE", required = true)]
    packages: Vec<String>,
}

/// Handle execution of `moss fetch`
#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let Command { output_dir, packages } = Command::from_arg_matches(args).unwrap();

    let verbose = args.get_flag("verbose");

    let mut client = Client::new(environment::NAME, installation)?;

    let packages = packages.iter().map(String::as_str).collect::<Vec<_>>();

    client.fetch(&packages, &output_dir, verbose)?;

    Ok(())
}
