// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::path::PathBuf;

use clap::Parser; // {ArgMatches, Command, arg, value_parser};

#[derive(Debug, Parser)]
#[command(about = "Index a collection of .stone packages")]
pub struct Command {
    #[arg(
        short,
        long = "inputdir",
        help = "The directory from which to start the index operation",
        default_value = ".",
        global = false
    )]
    pub inputdir: Option<PathBuf>,
    #[arg(
        short,
        long = "outputdir",
        help = "The directory to which to write the stone.index (defaults to {index_dir})",
        default_value = "{index_dir}",
        global = false
    )]
    pub outputdir: Option<PathBuf>,
}

pub use moss::client::index::Error;

pub fn handle(command: Command) -> Result<(), Error> {
    let Command { inputdir, outputdir } = command;

    let _inputdir = command.inputdir.clone();
    let _outputdir = command.outputdir.clone();

    moss::client::index(&_inputdir.unwrap(), _outputdir.as_deref())?;

    Ok(())
}
