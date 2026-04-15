// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use thiserror::Error;

use moss::{Client, Installation, client, environment};

#[derive(Debug, Parser)]
#[command(about = "Manage boot configuration via blsforme")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand, // NOTE: No Option, because a subcommand is required
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    #[command(about = "Show boot configuration status")]
    Status,
    #[command(about = "Synchronize boot configuration")]
    Sync,
}

/// Handle status for now
pub fn handle(command: Command, installation: Installation) -> Result<(), Error> {
    match command.subcommand {
        Subcommand::Status => status(installation),
        Subcommand::Sync => sync(installation),
    }
}

fn status(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::Client)?;

    client.print_boot_status()?;

    Ok(())
}

fn sync(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;

    client.synchronize_boot()?;

    println!("Boot updated\n");

    client.print_boot_status()?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
}
