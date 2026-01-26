// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};

use moss::{Installation, client::Client, environment};
use tracing::instrument;

pub use moss::client::Error;

pub fn command() -> Command {
    Command::new("remove")
        .visible_alias("rm")
        .about("Remove packages")
        .long_about("Remove packages by name")
        .arg(arg!(<NAME> ... "packages to remove").value_parser(clap::value_parser!(String)))
}

/// Handle execution of `moss remove`
#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let pkgs = args
        .get_many::<String>("NAME")
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let yes = *args.get_one::<bool>("yes").unwrap();

    let mut client = Client::new(environment::NAME, installation)?;

    client.remove(&pkgs, yes)?;

    Ok(())
}
