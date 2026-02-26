// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command};
use moss::{Client, Installation, client, environment};
use thiserror::Error;

pub fn command() -> Command {
    Command::new("cache")
        .about("Manage cached data")
        .subcommand_required(true)
        .subcommand(Command::new("prune").about("Prune cached artefacts").long_about(
            "Prune cached artefacts

This will remove all downloaded stones & unpacked asset data for packages not in any state or active repository.",
        ))
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    match args.subcommand() {
        Some(("prune", args)) => handle_prune(args, installation),
        _ => unreachable!(),
    }
}

fn handle_prune(_args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::SetupClient)?;

    let num_removed_files = client.prune_cache().map_err(Error::PruneCache)?;

    if num_removed_files > 0 {
        let s = if num_removed_files > 1 { "s" } else { "" };

        println!("{num_removed_files} file{s} removed");
    } else {
        println!("No files to remove");
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to setup moss client")]
    SetupClient(#[source] client::Error),
    #[error("failed to prune cache")]
    PruneCache(#[source] client::Error),
}
