// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};
use itertools::{Either, Itertools};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use thiserror::Error;

use moss::{
    Installation, Provider,
    client::{self, Client},
    environment,
    package::Flags,
    registry::transaction,
    state::Selection,
};
use tracing::{debug, info, info_span, instrument, warn};
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
#[instrument(skip_all)]
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let mut timing = Timing::default();
    let mut instant = Instant::now();

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
    let mut transaction = client.registry.transaction(transaction::Lookup::InstalledOnly)?;
    transaction.add(installed_ids.clone().into_iter().collect())?;

    // Remove all pkgs for removal
    transaction.remove(for_removal);

    // Finalized tx has all reverse deps removed
    let finalized = transaction.finalize().cloned().collect::<BTreeSet<_>>();

    // Resolve all removed packages, where removed is (installed - finalized)
    let removed = client.resolve_packages(installed_ids.difference(&finalized))?;

    timing.resolve = instant.elapsed();
    info!(
        total_packages = removed.len(),
        packages_to_remove = removed.len(),
        resolve_time_ms = timing.resolve.as_millis(),
        "Package resolution for removal completed"
    );

    debug!(count = removed.len(), "Full package list for removal");
    for package in &removed {
        debug!(
            name = %package.meta.name,
            version = %package.meta.version_identifier,
            source_release = package.meta.source_release,
            build_release = package.meta.build_release,
            "Package marked for removal"
        );
    }

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

    instant = Instant::now();

    let removal_span = info_span!("progress", phase = "removal", event_type = "progress");
    let _removal_guard = removal_span.enter();
    info!(
        phase = "removal",
        total_items = removed.len(),
        progress = 0.0,
        event_type = "progress_start",
    );

    // Print each package to stdout
    for package in &removed {
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
                        warn!(
                            package_id = ?id,
                            "Unreachable: previous selection not found during removal, marking as not explicit"
                        );

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

    timing.blit = instant.elapsed();
    info!(
        phase = "removal",
        duration_ms = timing.blit.as_millis(),
        items_processed = removed.len(),
        progress = 1.0,
        event_type = "progress_completed",
    );
    drop(_removal_guard);

    info!(
        blit_time_ms = timing.blit.as_millis(),
        total_time_ms = (timing.resolve + timing.blit).as_millis(),
        "Removal completed successfully"
    );

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

/// Simple timing information for Remove
#[derive(Default)]
pub struct Timing {
    pub resolve: Duration,
    pub blit: Duration,
}
