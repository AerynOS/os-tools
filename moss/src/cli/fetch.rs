// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::{Path, PathBuf};

use clap::{ArgMatches, Command, arg, value_parser};

use moss::{Installation, client::Client, environment};
use tracing::instrument;

pub use moss::client::Error;

pub fn command() -> Command {
    Command::new("fetch")
        .visible_alias("fe")
        .about("Fetch package(s)")
        .long_about("Fetch package stone(s) by name")
        .arg(arg!(<NAME> ... "packages to fetch").value_parser(clap::value_parser!(String)))
        .arg(
            arg!(-o --"output-dir" <OUTPUT_DIR> "directory to write the fetched stone(s) to (defaults to OUTPUT_DIR)")
                .value_parser(value_parser!(PathBuf)),
        )
}

/// Handle execution of `moss fetch`
#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let pkgs = args
        .get_many::<String>("NAME")
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();

    if let Some(dir) = args.get_one::<PathBuf>("output-dir")
        && !dir.exists()
    {
        std::fs::create_dir_all(dir)?;
    }

    let output_dir = match args.get_one::<PathBuf>("output-dir") {
        Some(dir) => &dir.canonicalize()?,
        None => &Path::new(".").canonicalize()?,
    };

    let mut client = Client::new(environment::NAME, installation)?;

    client.fetch(&pkgs, output_dir)?;

    Ok(())
}
