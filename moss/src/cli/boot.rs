// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command};
use thiserror::Error;

use moss::{Client, Installation, client, environment};

pub fn command() -> Command {
    Command::new("boot")
        .about("Boot management")
        .long_about("Manage boot configuration")
        .subcommand_required(true)
        .subcommand(Command::new("status").about("Status of boot configuration"))
        .subcommand(Command::new("sync").about("Synchronize boot configuration"))
}

/// Handle status for now
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    match args.subcommand() {
        Some(("status", args)) => status(args, installation),
        Some(("sync", args)) => sync(args, installation),
        _ => unreachable!(),
    }
}

fn status(_args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::Client)?;

    client.print_boot_status()?;

    Ok(())
}

fn sync(_args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;

    client.synchrnoize_boot()?;

    println!("Boot updated\n");

    client.print_boot_status()?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
}
