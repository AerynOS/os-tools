use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use itertools::Itertools;
use thiserror::Error;
use tracing::{Instrument, debug, info, info_span};
use tui::{
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::autoprint_columns,
};

use crate::{
    Client, Package, Provider, SystemModel, client, db, package, registry::transaction, runtime, state::Selection,
    system_model,
};

pub fn sync(client: &Client, import: Option<&Path>, yes: bool) -> Result<Timing, Error> {
    let mut timing = Timing::default();
    let mut instant = Instant::now();

    let system_model = if let Some(path) = import {
        Some(system_model::load(path)?.ok_or(Error::ImportSystemModelDoesntExist(path.to_owned()))?)
    } else {
        client.installation.system_model.clone()
    };

    // Grab all the existing installed packages
    let installed = client.registry.list_installed().collect::<Vec<_>>();

    // Resolve the final state of packages after considering sync updates
    let finalized = if let Some(system_model) = &system_model {
        resolve_with_system_model(client, system_model)?
    } else {
        resolve_with_installed(client, &installed)?
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
    let (added, updated): (Vec<_>, Vec<_>) = synced.iter().partition_map(|p| {
        if let Some(i) = installed.iter().find(|i| i.meta.name == p.meta.name)
            && !client.is_ephemeral()
        {
            itertools::Either::Right(package::Update { old: i, new: p })
        } else {
            itertools::Either::Left(*p)
        }
    });
    let removed = installed
        .iter()
        .filter(|p| !client.is_ephemeral() && !finalized.iter().any(|f| f.meta.name == p.meta.name))
        .cloned()
        .collect::<Vec<_>>();

    info!(
        added_packages = added.len(),
        upgraded_packages = updated.len(),
        removed_packages = removed.len(),
        "Sync analysis completed"
    );

    if synced.is_empty() && removed.is_empty() {
        println!("No packages to sync");
        return Ok(timing);
    }

    if !added.is_empty() {
        println!("The following packages will be added: ");
        println!();
        autoprint_columns(added.as_slice());
        println!();
    }
    if !updated.is_empty() {
        println!("The following packages will be updated: ");
        println!();
        autoprint_columns(updated.as_slice());
        println!();
    }
    if !removed.is_empty() {
        println!("The following orphaned packages will be removed: ");
        println!();
        autoprint_columns(removed.as_slice());
        println!();
    }

    // Must we prompt?
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

    Ok(timing)
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
    DB(#[from] db::Error),

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
