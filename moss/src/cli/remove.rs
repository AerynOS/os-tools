// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};
use itertools::{Either, Itertools};
use std::collections::BTreeSet;
use thiserror::Error;

use moss::{
    Installation, Provider,
    client::{self, Client},
    environment,
    package::Flags,
    registry::transaction,
    state::Selection,
};
use tui::{
    Styled,
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::autoprint_columns,
};

pub fn command() -> Command {
    Command::new("remove")
        .visible_alias("rm")
        .about("Remove packages")
        .long_about("Remove packages by name")
        .arg(arg!(<NAME> ... "packages to remove").value_parser(clap::value_parser!(String)))
}

/// Handle execution of `moss remove`
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let pkgs = args
        .get_many::<String>("NAME")
        .into_iter()
        .flatten()
        .map(|name| Provider::from_name(name).unwrap())
        .collect::<Vec<_>>();
    let yes = *args.get_one::<bool>("yes").unwrap();

    // Grab a client for the target, enumerate packages
    let client = Client::new(environment::NAME, installation)?;

    let installed = client.registry.list_installed(Flags::default()).collect::<Vec<_>>();
    let installed_ids = installed.iter().map(|p| p.id.clone()).collect::<BTreeSet<_>>();

    // Separate packages between installed / not installed (or invalid)
    let (for_removal, not_installed): (Vec<_>, Vec<_>) = pkgs.iter().partition_map(|provider| {
        installed
            .iter()
            .find(|i| i.meta.providers.contains(provider))
            .map(|i| Either::Left(i.id.clone()))
            .unwrap_or(Either::Right(provider.clone()))
    });

    // Bail if there's packages not installed
    // TODO: Add error hookups
    if !not_installed.is_empty() {
        println!("Missing packages in lookup: {not_installed:?}");
        return Err(Error::NoSuchPackage);
    }

    // Add all installed packages to transaction
    let mut transaction = client
        .registry
        .transaction_with_installed(installed_ids.iter().cloned().collect())?;

    // Remove all pkgs for removal
    transaction.remove(for_removal);

    // Finalized tx has all reverse deps removed
    let finalized = transaction.finalize().cloned().collect::<BTreeSet<_>>();

    // Resolve all removed packages, where removed is (installed - finalized)
    let removed = client.resolve_packages(installed_ids.difference(&finalized))?;

    println!("The following package(s) will be removed:");
    println!();
    autoprint_columns(&removed);
    println!();

    let result = if yes {
        true
    } else {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(" Do you wish to continue? ")
            .default(false)
            .interact()?
    };
    if !result {
        return Err(Error::Cancelled);
    }

    // Print each package to stdout
    for package in removed {
        println!("{} {}", "Removed".red(), package.meta.name.to_string().bold());
    }

    // Map finalized state to a [`Selection`] by referencing
    // it's value from the previous state
    let new_state_pkgs = {
        let previous_selections = match client.installation.active_state {
            Some(id) => client.state_db.get(id)?.selections,
            None => vec![],
        };

        finalized
            .into_iter()
            .map(|id| {
                previous_selections
                    .iter()
                    .find(|s| s.package == id)
                    .cloned()
                    // Should be unreachable since new state from removal
                    // is always a subset of the previous state
                    .unwrap_or_else(|| {
                        eprintln!("Unreachable: previous selection not found during removal for package {id:?}, marking as not explicit");

                        Selection {
                            package: id,
                            explicit: false,
                            reason: None,
                        }
                    })
            })
            .collect::<Vec<_>>()
    };

    // Apply state
    client.new_state(&new_state_pkgs, "Remove")?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,

    #[error("no such package")]
    NoSuchPackage,

    #[error("client")]
    Client(#[from] client::Error),

    #[error("transaction")]
    Transaction(#[from] transaction::Error),

    #[error("db")]
    DB(#[from] moss::db::Error),

    #[error("io")]
    Io(#[from] std::io::Error),

    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),
}
