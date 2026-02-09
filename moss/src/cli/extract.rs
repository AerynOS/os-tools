// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{ArgMatches, Command, arg};

pub use moss::client::extract::Error;

pub fn command() -> Command {
    Command::new("extract")
        .about("Extract a `.stone` content to disk")
        .long_about("For all valid content-bearing archives, extract to disk")
        .arg(arg!(<PATH> ... "files to extract").value_parser(clap::value_parser!(PathBuf)))
        .arg(
            arg!(-o --"output-dir" <OUTPUT_DIR> "directory to extract the stone(s) to")
                .default_value(".")
                .value_parser(clap::value_parser!(PathBuf)),
        )
}

/// Handle the `extract` command
pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let paths = args
        .get_many::<PathBuf>("PATH")
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    let output_dir = args.get_one::<PathBuf>("output-dir").unwrap();

    moss::client::extract(paths, output_dir)?;

    Ok(())
}
