// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use moss::registry::transaction;
use moss::state::Selection;
use moss::{Installation, Provider, SystemModel, environment, runtime, system_model};
use moss::{
    Package,
    client::{self, Client},
    package::{self},
};
use thiserror::Error;

use tracing::{Instrument, debug, info, info_span, instrument};
use tui::dialoguer::Confirm;
use tui::dialoguer::theme::ColorfulTheme;
use tui::pretty::autoprint_columns;

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

    let mut timing = Timing::default();
    let mut instant = Instant::now();

    let yes_all = *args.get_one::<bool>("yes").unwrap();
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

    let system_model = if let Some(path) = command.import {
        Some(system_model::load(&path)?.ok_or(Error::ImportSystemModelDoesntExist(path))?)
    } else {
        client.installation.system_model.clone()
    };

    // Grab all the existing installed packages
    let installed = client.registry.list_installed().collect::<Vec<_>>();

    // Resolve the final state of packages after considering sync updates
    let finalized = if let Some(system_model) = &system_model {
        resolve_with_system_model(&client, system_model)?
    } else {
        resolve_with_installed(&client, &installed)?
    };
    debug!(count = finalized.len(), "Full package list after sync");
    for package in &finalized {
        debug!(
            name = %package.meta.name,
            version = %package.meta.version_identifier,
            source_release = package.meta.source_release,
            build_release = package.meta.build_release,
            "Package in finalized list"
        );
    }

    timing.resolve = instant.elapsed();
    info!(
        total_resolved = finalized.len(),
        resolve_time_ms = timing.resolve.as_millis(),
        "Package resolution completed"
    );

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

    info!(
        synced_packages = synced.len(),
        removed_packages = removed.len(),
        "Sync analysis completed"
    );

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

    instant = Instant::now();

    let cache_packages_span = info_span!("progress", phase = "cache_packages", event_type = "progress");
    let _cache_packages_guard = cache_packages_span.enter();
    info!(
        total_items = synced.len(),
        progress = 0.0,
        event_type = "progress_start"
    );

    runtime::block_on(client.cache_packages(&synced).in_current_span())?;

    timing.fetch = instant.elapsed();
    info!(
        duration_ms = timing.fetch.as_millis(),
        items_processed = synced.len(),
        progress = 1.0,
        event_type = "progress_completed",
    );
    drop(_cache_packages_guard);
    instant = Instant::now();

    let new_selections = if let Some(system_model) = &system_model {
        // For system model, "explicit" is what was defined in the system model file

        finalized
            .into_iter()
            .map(|p| {
                let is_explicit = system_model.packages.intersection(&p.meta.providers).next().is_some();

                Selection {
                    package: p.id,
                    explicit: is_explicit,
                    // TODO: We can map the "why" of system-model packages to this? Or
                    // can we remove "reason" entirely, we haven't used it to-date
                    reason: None,
                }
            })
            .collect()
    } else {
        // Map finalized state to a [`Selection`] by referencing it's value from the previous state
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
    client.new_state(&new_selections, "Sync")?;

    timing.blit = instant.elapsed();

    info!(
        blit_time_ms = timing.blit.as_millis(),
        total_time_ms = (timing.resolve + timing.fetch + timing.blit).as_millis(),
        "Sync completed successfully"
    );

    Ok(())
}

/// Returns the resolved package set w/ sync'd changes swapped in using
/// the provided installed `packages`
///
/// Used to sync in "implicit" mode, where the active state is the source of truth
#[tracing::instrument(skip_all)]
fn resolve_with_installed(client: &Client, packages: &[Package]) -> Result<Vec<Package>, Error> {
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
                && !all_ids.contains(&lookup.id)
            {
                return Some(lookup.id);
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

/// Returns the resolved package set based on the packages defined in the system model
///
/// System model is the source of truth here vs "implicit" mode which relies on the active
/// state + configured repos as the source of truth
#[tracing::instrument(skip_all)]
fn resolve_with_system_model(client: &Client, system_model: &SystemModel) -> Result<Vec<Package>, Error> {
    // Lookup the available package for each
    let packages = system_model
        .packages
        .iter()
        .map(|provider| {
            client
                .registry
                .by_provider_id_only(provider, package::Flags::default().with_available())
                .next()
                .ok_or(Error::MissingSystemModelPackage(provider.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Add them to a transaction that only resolves transitives from available repositories
    let mut tx = client.registry.transaction(transaction::Lookup::AvailableOnly)?;
    tx.add(packages)?;

    // Resolve the tx
    Ok(client.resolve_packages(tx.finalize())?)
}

/// Simple timing information for Sync
#[derive(Default)]
pub struct Timing {
    pub resolve: Duration,
    pub fetch: Duration,
    pub blit: Duration,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Package defined in system-model does not exist in any repository: {0}")]
    MissingSystemModelPackage(Provider),

    #[error("cancelled")]
    Cancelled,

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

    #[error("load system model")]
    LoadSystemModel(#[from] system_model::LoadError),

    #[error("system model doesn't exist at {0:?}")]
    ImportSystemModelDoesntExist(PathBuf),
}
