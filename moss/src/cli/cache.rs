// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use moss::{Client, Installation, client, environment};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(about = "Managed cached data")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

impl Command {
    pub fn handle(self, installation: Installation) -> Result<(), Error> {
        match self.subcommand {
            Subcommand::Prune => prune(installation),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to setup moss client")]
    SetupClient(#[source] client::Error),
    #[error("failed to prune cache")]
    PruneCache(#[source] client::Error),
}

#[derive(Debug, clap::Subcommand)]

enum Subcommand {
    #[command(
        about = concat!(
            "Prune cached artefacts. ",
            "This will remove all downloaded stones and unpacked asset data ",
            "for packages not in any state or active repository")
    )]
    Prune,
}

fn prune(installation: Installation) -> Result<(), Error> {
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
