// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};

use clap_complete::ArgValueCompleter;
use moss::dynamic_completion::prefix_completer;
use moss::{Installation, client::Client, environment, package};
use tracing::instrument;

pub use moss::client::Error;

pub fn command() -> clap::Command {
    Command::command()
}

#[derive(Debug, Parser)]
#[command(
    name = "remove",
    visible_alias = "rm",
    about = "Remove packages",
    long_about = "Remove packages by name"
)]
pub struct Command {
    /// Packages to remove
    #[arg(add = ArgValueCompleter::new(prefix_completer(package::Flags::default().with_installed())))]
    packages: Vec<String>,

    /// Simulate the operation (dry-run)
    #[arg(long)]
    dry_run: bool,
}

/// Handle execution of `moss remove`
#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let command = Command::from_arg_matches(args).expect("validated by clap");

    let pkgs = command.packages.iter().map(String::as_str).collect::<Vec<_>>();
    let yes = *args.get_one::<bool>("yes").unwrap();
    let simulate = command.dry_run;

    let mut client = Client::new(environment::NAME, installation)?;

    client.remove(&pkgs, yes, simulate)?;

    Ok(())
}
