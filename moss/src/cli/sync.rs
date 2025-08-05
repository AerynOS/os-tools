// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{ArgMatches, Command, arg, value_parser};
use moss::registry::transaction;
use moss::state::Selection;
use moss::{Installation, environment, runtime};
use moss::{
    Package,
    client::{self, Client},
    package::{self},
};
use thiserror::Error;

use tui::dialoguer::Confirm;
use tui::dialoguer::theme::ColorfulTheme;
use tui::pretty::autoprint_columns;

pub fn command() -> Command {
    Command::new("sync")
        .visible_alias("up")
        .about("Sync packages")
        .long_about("Sync package selections with candidates from the highest priority repository")
        .arg(arg!(-u --"update" "Update repositories before syncing"))
        .arg(arg!(--"upgrade-only" "Only sync packages that have a version upgrade"))
        .arg(
            arg!(--to <blit_target> "Blit this sync to the provided directory instead of the root")
                .long_help(
                    "Blit this sync to the provided directory instead of the root. \n\
                     \n\
                     This operation won't be captured as a new state",
                )
                .value_parser(value_parser!(PathBuf)),
        )
}

pub fn handle(args: &ArgMatches, installation: Installation, debug: bool) -> Result<(), Error> {
    let yes_all = *args.get_one::<bool>("yes").unwrap();
    let update = *args.get_one::<bool>("update").unwrap();
    let upgrade_only = *args.get_one::<bool>("upgrade-only").unwrap();

    let mut client = Client::new(environment::NAME, installation)?;

    // Make ephemeral if a blit target was provided
    if let Some(blit_target) = args.get_one::<PathBuf>("to").cloned() {
        client = client.ephemeral(blit_target)?;
    }

    // Update repos if requested
    if update {
        runtime::block_on(client.refresh_repositories())?;
    }

    // Grab all the existing installed packages
    let installed = client
        .registry
        .list_installed(package::Flags::default())
        .collect::<Vec<_>>();
    if installed.is_empty() {
        return Err(Error::NoInstall);
    }

    // Resolve the final state of packages after considering sync updates
    let finalized = resolve_with_sync(&client, upgrade_only, &installed)?;
    if debug {
        println!("Full package list after sync: ");
        println!();
        autoprint_columns(&finalized);
        println!();
    }

    // Synced are packages are:
    //
    // Stateful: Not installed
    // Ephemeral: All
    let synced = finalized
        .iter()
        .filter(|p| client.is_ephemeral() || !installed.iter().any(|i| i.id == p.id))
        .collect::<Vec<_>>();
    let removed = installed
        .iter()
        .filter(|p| !finalized.iter().any(|f| f.meta.name == p.meta.name))
        .cloned()
        .collect::<Vec<_>>();

    if synced.is_empty() && removed.is_empty() {
        println!("No packages to sync");
        return Ok(());
    }

    if !synced.is_empty() {
        println!("The following packages will be sync'd: ");
        println!();
        autoprint_columns(synced.as_slice());
        println!();
    }
    if !removed.is_empty() {
        println!("The following orphaned packages will be removed: ");
        println!();
        autoprint_columns(removed.as_slice());
        println!();
    }

    // Must we prompt?
    let result = if yes_all {
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

    runtime::block_on(client.cache_packages(&synced))?;

    // Map finalized state to a [`Selection`] by referencing
    // it's value from the previous state
    let new_selections = {
        let previous_selections = match client.installation.active_state {
            Some(id) => client.state_db.get(id)?.selections,
            None => vec![],
        };

        finalized
            .into_iter()
            .map(|p| {
                // Use old version id to lookup previous selection
                let lookup_id = installed
                    .iter()
                    .find_map(|i| (i.meta.name == p.meta.name).then_some(&i.id))
                    .unwrap_or(&p.id);

                previous_selections
                    .iter()
                    .find(|s| s.package == *lookup_id)
                    .cloned()
                    // Use prev reason / explicit flag & new id
                    .map(|s| Selection {
                        package: p.id.clone(),
                        ..s
                    })
                    // Must be transitive
                    .unwrap_or(Selection {
                        package: p.id,
                        explicit: false,
                        reason: None,
                    })
            })
            .collect::<Vec<_>>()
    };

    // Perfect, apply state.
    client.new_state(&new_selections, "Sync", None)?;

    Ok(())
}

/// Returns the resolved package set w/ sync'd changes swapped in using
/// the provided `packages`
#[tracing::instrument(skip_all, fields(upgrade_only = upgrade_only))]
fn resolve_with_sync(client: &Client, upgrade_only: bool, packages: &[Package]) -> Result<Vec<Package>, Error> {
    let all_ids = packages.iter().map(|p| &p.id).collect::<BTreeSet<_>>();

    // For each explicit package, replace it w/ it's sync'd change (if available)
    // or return the original package
    let with_sync = packages
        .iter()
        .filter_map(|p| {
            if !p.flags.explicit {
                return None;
            }

            // Get first available = use highest priority
            if let Some(lookup) = client
                .registry
                .by_name(&p.meta.name, package::Flags::new().with_available())
                .next()
            {
                let upgrade_check = if upgrade_only {
                    lookup.meta.source_release > p.meta.source_release
                } else {
                    true
                };

                if !all_ids.contains(&lookup.id) && upgrade_check {
                    return Some(lookup.id);
                }
            }

            Some(p.id.clone())
        })
        .collect::<Vec<_>>();

    // Build a new tx from this sync'd package set
    let mut tx = client.registry.transaction(transaction::Lookup::PreferAvailable)?;
    // Add all explicit packages to build the final tx state
    tx.add(with_sync)?;

    // Resolve the tx
    Ok(client.resolve_packages(tx.finalize())?)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,

    #[error("no installation")]
    NoInstall,

    #[error("client")]
    Client(#[from] client::Error),

    #[error("db")]
    DB(#[from] moss::db::Error),

    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),

    #[error("transaction")]
    Transaction(#[from] transaction::Error),

    #[error("io")]
    Io(#[from] std::io::Error),
}
