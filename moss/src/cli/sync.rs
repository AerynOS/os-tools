// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use moss::{Installation, client::Client, environment, runtime};
use tracing::instrument;

pub use moss::client::Error;

pub fn command() -> clap::Command {
    Command::command()
}

#[derive(Debug, Parser)]
#[command(
    name = "sync",
    visible_alias = "up",
    about = "Sync packages",
    long_about = "Sync package selections with candidates from the highest priority repository"
)]
pub struct Command {
    /// Update repositories before syncing
    #[arg(short, long)]
    update: bool,
    /// Blit this sync to the provided directory instead of the root
    ///
    /// This operation won't be captured as a new state
    #[arg(value_name = "dir", long = "to")]
    blit_target: Option<PathBuf>,

    /// Sync against the provided system-model.kdl
    ///
    /// Only the repositories and packages from the provided file
    /// will be used to create the new state
    #[arg(value_name = "file", long)]
    import: Option<PathBuf>,
}

#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let command = Command::from_arg_matches(args).expect("validated by clap");

    let yes = *args.get_one::<bool>("yes").unwrap();
    let update = command.update;

    let mut client = Client::new(environment::NAME, installation)?;

    // Make ephemeral if a blit target was provided
    if let Some(blit_target) = command.blit_target {
        client = client.ephemeral(blit_target)?;
    }

    // Update repos if requested
    if update {
        runtime::block_on(client.refresh_repositories())?;
    }

    client.sync(command.import.as_deref(), yes)?;

    Ok(())
}
