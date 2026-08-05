// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use moss::{Client, Installation, client, environment};

#[derive(Debug, Parser)]
#[command(about = "Manage boot configuration via blsforme")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

impl Command {
    pub fn handle(self, installation: Installation) -> Result<(), client::Error> {
        match self.subcommand {
            Subcommand::Status => status(installation),
            Subcommand::Sync => sync(installation),
        }
    }
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    #[command(about = "Show boot configuration status")]
    Status,
    #[command(about = "Synchronize boot configuration")]
    Sync,
}

fn status(installation: Installation) -> Result<(), client::Error> {
    let client = Client::new(environment::NAME, installation)?;

    client.print_boot_status()?;

    Ok(())
}

fn sync(installation: Installation) -> Result<(), client::Error> {
    let client = Client::new(environment::NAME, installation)?;

    client.synchronize_boot()?;

    println!("Boot updated\n");

    client.print_boot_status()?;

    Ok(())
}
