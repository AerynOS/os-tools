use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use itertools::{Either, Itertools};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};
use tui::{
    Styled,
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::autoprint_columns,
};

use crate::{Client, Provider, client, db, registry::transaction, state::Selection};

/// Remove a set of packages.
#[instrument(skip(client), fields(ephemeral = client.is_ephemeral()))]
pub fn remove(client: &mut Client, pkgs: &[&str], yes: bool) -> Result<Timing, Error> {
    let mut timing = Timing::default();
    let mut instant = Instant::now();

    let installed = client.registry.list_installed().collect::<Vec<_>>();
    let installed_ids = installed.iter().map(|p| p.id.clone()).collect::<BTreeSet<_>>();

    // Separate packages between installed / not installed (or invalid)
    let (for_removal, not_installed): (Vec<_>, Vec<_>) = pkgs.iter().partition_map(|name| {
        let provider = Provider::from_name(name).unwrap();

        installed
            .iter()
            .find(|i| i.meta.providers.contains(&provider))
            .map(|i| Either::Left(i.id.clone()))
            .unwrap_or(Either::Right(provider.clone()))
    });

    // Bail if there's packages not installed
    // TODO: Add error hookups
    if !not_installed.is_empty() {
        println!("Missing packages in lookup: {not_installed:?}");
        return Err(Error::NoSuchPackage);
    }

    // First resolve a transaction where all requested packages are removed from the install
    //
    // This will remove those packages & any package that depends on it. This will not remove
    // the packages it depends on if they are orphaned (see next step).
    let tx_with_removed = {
        // Add all installed packages to transaction
        let mut transaction = client.registry.transaction(transaction::Lookup::InstalledOnly)?;
        transaction.add(installed_ids.clone().into_iter().collect())?;

        // Remove all pkgs for removal
        transaction.remove(for_removal);

        // Finalized tx has all reverse deps removed
        transaction.finalize().cloned().collect::<BTreeSet<_>>()
    };

    // Build a new transaction w/ the leftover "explicit" packages. This will cause all orphaned
    // transitive dependencies to get dropped. These are packages that were depended on by removed
    // packages that are no longer depended on.
    let finalized = {
        // Is an explicit package that still exists after removals
        let explicit_pkgs = installed
            .iter()
            .filter(|p| tx_with_removed.contains(&p.id) && p.flags.explicit)
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();

        let mut transaction = client.registry.transaction(transaction::Lookup::InstalledOnly)?;
        transaction.add(explicit_pkgs)?;

        transaction.finalize().cloned().collect::<BTreeSet<_>>()
    };

    // Resolve all removed packages, where removed is (installed - finalized)
    let removed = client.resolve_packages(installed_ids.difference(&finalized))?;

    timing.resolve = instant.elapsed();
    info!(
        total_packages = removed.len(),
        packages_to_remove = removed.len(),
        resolve_time_ms = timing.resolve.as_millis(),
        "Package resolution for removal completed"
    );

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

    // Print each package to stdout
    for package in &removed {
        println!("{} {}", "Removed".red(), package.meta.name.as_str().bold());
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
        blit_time_ms = timing.blit.as_millis(),
        total_time_ms = (timing.resolve + timing.blit).as_millis(),
        "Removal completed successfully"
    );

    Ok(timing)
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
    DB(#[from] db::Error),

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
