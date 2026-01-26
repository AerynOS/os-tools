// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::path::PathBuf;

use clap::{ArgMatches, Command, arg, value_parser};

pub use moss::client::index::Error;

pub fn command() -> Command {
    Command::new("index")
        .visible_alias("ix")
        .about("Index a collection of packages")
        .arg(arg!(<INDEX_DIR> "directory of index files").value_parser(value_parser!(PathBuf)))
        .arg(
            arg!(-o --"output-dir" [output_dir] "directory to write the stone.index to (defaults to INDEX_DIR)")
                .value_parser(value_parser!(PathBuf)),
        )
}

pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let index_dir = args.get_one::<PathBuf>("INDEX_DIR").unwrap().canonicalize()?;
    let output_dir = args
        .get_one::<PathBuf>("output-dir")
        .map(|dir| dir.canonicalize())
        .transpose()?;

    moss::client::index(&index_dir, output_dir.as_deref())?;

    Ok(())
}
