// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::Parser;
use moss::client::index::Error;

#[derive(Debug, Parser)]
#[command(about = "Index a collection of .stone packages")]
pub struct Command {
    #[arg(
        short,
        long = "input-dir",
        help = "The directory from which to start the index operation",
        default_value = ".",
        global = false
    )]
    input_dir: PathBuf,

    #[arg(
        short,
        long = "output-dir",
        help = "The directory to which to write the stone.index (defaults to INPUT-DIR)",
        global = false
    )]
    output_dir: Option<PathBuf>,
}

impl Command {
    pub fn handle(self) -> Result<(), Error> {
        moss::client::index(&self.input_dir, self.output_dir.as_deref())?;
        Ok(())
    }
}
