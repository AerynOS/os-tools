// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{ArgMatches, Command};
use thiserror::Error;

use moss::{Client, Installation, client, db, environment, state};

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
    client::boot::status(&installation).map_err(Error::BootStatus)
}

fn sync(_args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::InitClient)?;

    let Some(state_id) = client.installation.active_state else {
        return Err(Error::NoActiveState(client.installation.root));
    };

    let state = client
        .state_db
        .get(state_id)
        .map_err(|err| Error::LoadStateDb(err, state_id))?;

    client::boot::synchronize(&client, &state).map_err(Error::SyncBoot)?;

    println!("Boot updated\n");

    client::boot::status(&client.installation).map_err(Error::BootStatus)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("initialize client")]
    InitClient(#[source] client::Error),

    #[error("No active moss state found under {0:?}")]
    NoActiveState(PathBuf),

    #[error("load state {0} from db")]
    LoadStateDb(#[source] db::Error, state::Id),

    #[error("synchronize boot")]
    SyncBoot(#[source] client::boot::Error),

    #[error("boot status")]
    BootStatus(#[source] client::boot::Error),
}
