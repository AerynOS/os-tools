// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! The pruning system for moss states and assets
//!
//! Quite simply this is a strategy based garbage collector for unused/unwanted
//! system states (i.e. historical snapshots) that cleans up database entries
//! and assets on disk by way of refcounting.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err as fs;
use itertools::Itertools;
use thiserror::Error;

use tracing::info;
use tui::{ProgressBar, ProgressStyle, Styled};
use tui::{
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::autoprint_columns,
};

use crate::client::boot;
use crate::util;
use crate::{Client, Installation, State, client::cache, db, package, repository, state};

/// The prune strategy for removing old states
#[derive(Debug, Clone, Copy)]
pub enum Strategy<'a> {
    /// Keep the most recent N states, remove the rest
    KeepRecent { keep: u64, include_newer: bool },
    /// Removes state(s)
    Remove(&'a [state::Id]),
}

/// Prune old states using [`Strategy`] and garbage collect
/// all cached data related to those states being removed
pub(super) fn prune_states(client: &Client, strategy: Strategy<'_>, yes: bool) -> Result<(), Error> {
    let installation = &client.installation;
    let layout_db = &client.layout_db;
    let state_db = &client.state_db;
    let install_db = &client.install_db;

    let mut timing = Timing::default();
    let mut instant = Instant::now();

    // Only prune if the moss root has an active state (otherwise
    // it's probably borked or not setup yet)
    let Some(current_state_id) = installation.active_state else {
        return Err(Error::NoActiveState);
    };
    let current_state = state_db.get(current_state_id)?;

    let state_ids = state_db.list_ids()?;

    // Find each state we need to remove
    let removal_ids = match strategy {
        Strategy::KeepRecent { keep, include_newer } => {
            // Filter for all removal candidates
            let candidates = state_ids
                .iter()
                .filter(|(id, _)| {
                    if include_newer {
                        *id != current_state.id
                    } else {
                        *id < current_state.id
                    }
                })
                .collect::<Vec<_>>();
            // Deduct current state from num candidates to keep
            let candidate_limit = (keep as usize).saturating_sub(1);

            // Calculate how many candidate states over the limit we are
            let num_to_remove = candidates.len().saturating_sub(candidate_limit);

            // Sort ascending and assign first `num_to_remove` as `Status::Remove`
            candidates
                .into_iter()
                .sorted_by_key(|(_, created)| *created)
                .enumerate()
                .filter_map(|(idx, (id, _))| if idx < num_to_remove { Some(*id) } else { None })
                .collect::<Vec<_>>()
        }
        Strategy::Remove(remove) => state_ids
            .iter()
            .filter_map(|(id, _)| remove.contains(id).then_some(*id))
            .collect(),
    };

    // Bail if there's no states to remove
    if removal_ids.is_empty() {
        // TODO: Print no states to be removed
        return Ok(());
    }

    // Keep track of how many active states are using a package
    let mut packages_counts = BTreeMap::<package::Id, usize>::new();
    let mut removals = vec![];

    // Get net refcount of each package in all states
    for (id, _) in state_ids {
        // Get metadata
        let state = state_db.get(id)?;

        // Increment each package
        for selection in &state.selections {
            *packages_counts.entry(selection.package.clone()).or_default() += 1;
        }

        // Decrement if removal
        if removal_ids.contains(&id) {
            // Ensure we're not pruning the active state!!
            if id == current_state.id {
                return Err(Error::PruneCurrent);
            }

            for selection in &state.selections {
                *packages_counts.entry(selection.package.clone()).or_default() -= 1;
            }
            removals.push(state);
        }
    }

    // Get all packages which were decremented to 0,
    // these are the packages we want to remove since
    // no more states reference them
    let package_removals = packages_counts
        .into_iter()
        .filter_map(|(pkg, count)| (count == 0).then_some(pkg))
        .collect::<Vec<_>>();

    timing.resolve = instant.elapsed();
    info!(
        total_resolved_states = removals.len(),
        total_resolved_packages = package_removals.len(),
        resolve_time_ms = timing.resolve.as_millis(),
        "Resolved states marked for removal"
    );
    instant = Instant::now();

    // Print out the states to be removed to the user
    println!("The following state(s) will be removed:");
    println!();
    autoprint_columns(&removals.iter().map(state::ColumnDisplay).collect::<Vec<_>>());
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

    // Prune these states / packages from all dbs
    prune_databases(&removals, &package_removals, state_db, install_db, layout_db)?;

    timing.prune_db = instant.elapsed();
    info!(
        prune_db_time_ms = timing.prune_db.as_millis(),
        "Pruned stale packages & states from databases"
    );
    instant = Instant::now();

    // Remove orphaned downloads
    remove_orphaned_files(
        // root
        installation.cache_path("downloads").join("v1"),
        // final set of hashes to compare against
        install_db.file_hashes()?,
        // path builder using hash
        |hash| cache::download_path(installation, &hash).ok(),
    )?;

    // Remove orphaned assets
    remove_orphaned_files(
        // root
        installation.assets_path("v2"),
        // final set of hashes to compare against
        layout_db.file_hashes()?,
        // path builder using hash
        |hash| Some(cache::asset_path(installation, &hash)),
    )?;

    timing.orphaned_files = instant.elapsed();
    info!(
        orphaned_file_time_ms = timing.orphaned_files.as_millis(),
        "Removed orphaned files"
    );
    instant = Instant::now();

    let archive_paths = removals
        .iter()
        .map(|s| installation.root_path(s.id.to_string()))
        .collect::<Vec<_>>();

    info!(
        total_archived_paths = archive_paths.len(),
        progress = 0.0,
        event_type = "progress_start",
        "Removing stale archive trees"
    );

    remove_archived_states(&archive_paths)?;

    timing.prune_archives = instant.elapsed();
    info!(
        duration_ms = timing.prune_archives.as_millis(),
        items_processed = archive_paths.len(),
        progress = 1.0,
        event_type = "progress_completed",
    );

    // Sync boot to ensure pruned states are removed from boot entries
    boot::synchronize(client, &current_state).map_err(Error::SyncBoot)?;

    Ok(())
}

/// Prune all cached data that isn't related to any states
/// or active repositories. This will remove all downloaded
/// stones & unpacked asset data for packages not in that set.
///
/// # Arguments
///
/// * - `state_db`     - Installation's state database
/// * - `install_db`   - Installation's "installed" database
/// * - `layout_db`    - Installation's layout database
/// * - `installation` - Client specific target filesystem encapsulation
/// * - `repositories` - All configured repositories
pub(super) fn prune_cache(
    state_db: &db::state::Database,
    install_db: &db::meta::Database,
    layout_db: &db::layout::Database,
    installation: &Installation,
    repositories: &repository::Manager,
) -> Result<usize, Error> {
    // Prune all packages from our internal DBs that aren't
    // part of a state or an active repository
    {
        // Packages in all states (active + archived)
        let state_packages = state_db
            .all()?
            .into_iter()
            .flat_map(|state| state.selections.into_iter().map(|selection| selection.package))
            .collect::<BTreeSet<_>>();

        // Packages in all active repos
        let repo_packages = repositories
            .active()
            .map(|repo| repo.db.package_ids())
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>();

        // Keep state + active repo packages
        let packages_to_keep = state_packages.into_iter().chain(repo_packages).collect::<BTreeSet<_>>();

        // Prune packages not in `packages_to_keep` from layout db (layout entries)
        {
            let layout_packages = layout_db.package_ids()?;
            let to_remove = layout_packages.difference(&packages_to_keep);
            layout_db.batch_remove(to_remove)?;
        }

        // Prune packages not in `packages_to_keep` from install db (meta entries)
        {
            let install_packages = install_db.package_ids()?;
            let to_remove = install_packages.difference(&packages_to_keep);
            install_db.batch_remove(to_remove)?;
        }
    }

    let mut num_removed_files = 0;

    // Now we can prune "orphaned package artefacts" / packages artefacts
    // on disk but not defined in our internal dbs
    {
        // Remove orphaned downloads (package stones)
        num_removed_files += remove_orphaned_files(
            // root
            installation.cache_path("downloads").join("v1"),
            // final set of hashes to compare against
            install_db.file_hashes()?,
            // path builder using hash
            |hash| cache::download_path(installation, &hash).ok(),
        )?;

        // Remove orphaned assets (unpacked package assets in CAS)
        num_removed_files += remove_orphaned_files(
            // root
            installation.assets_path("v2"),
            // final set of hashes to compare against
            layout_db.file_hashes()?,
            // path builder using hash
            |hash| Some(cache::asset_path(installation, &hash)),
        )?;
    }

    Ok(num_removed_files)
}

/// Removes the provided states & packages from the databases
/// When any removals cause a filesystem asset to become completely unreffed
/// it will be permanently deleted from disk.
///
/// # Arguments
///
/// * `states`     - The states to prune from the DB
/// * `packages`   - any packages to prune from the DB
/// * `state_db`   - Client State database
/// * `install_db` - Client "installed" database
/// * `layout_db`  - Client layout database
fn prune_databases(
    states: &[State],
    packages: &[package::Id],
    state_db: &db::state::Database,
    install_db: &db::meta::Database,
    layout_db: &db::layout::Database,
) -> Result<(), Error> {
    // Remove db states
    state_db.batch_remove(states.iter().map(|s| &s.id))?;
    // Remove db metadata
    install_db.batch_remove(packages)?;
    // Remove db layouts
    layout_db.batch_remove(packages)?;

    Ok(())
}

/// Removes all files under `root` that no longer exist in the provided `final_hashes` set
fn remove_orphaned_files(
    root: PathBuf,
    final_hashes: BTreeSet<String>,
    compute_path: impl Fn(String) -> Option<PathBuf>,
) -> Result<usize, Error> {
    // Compute hashes to remove by (installed - final)
    let installed_hashes = enumerate_file_hashes(&root)?;
    let hashes_to_remove = installed_hashes.difference(&final_hashes);

    // Remove each and it's parent dir if empty
    hashes_to_remove.into_iter().try_fold(0, |acc, hash| {
        // Compute path to file using hash
        let Some(file) = compute_path(hash.clone()) else {
            return Ok(acc);
        };
        let partial = file.with_added_extension("part");

        // Remove if it exists
        if file.exists() {
            fs::remove_file(&file)?;
        }

        // Remove partial file if it exists
        if partial.exists() {
            fs::remove_file(&partial)?;
        }

        // Try to remove leading parent dirs if they're
        // now empty
        if let Some(parent) = file.parent() {
            let _ = remove_empty_dirs(parent, &root);
        }

        Ok(acc + 1)
    })
}

/// Returns all nested files under `root` and parses the file name as a hash
fn enumerate_file_hashes(root: impl AsRef<Path>) -> io::Result<BTreeSet<String>> {
    let files = enumerate_files(root)?;

    let path_to_hash = |path: PathBuf| path.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_owned();

    Ok(files.into_iter().map(path_to_hash).collect())
}

/// Returns all nested files under `root`
fn enumerate_files(root: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    use rayon::prelude::*;

    fn recurse(dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        let mut dirs = vec![];
        let mut files = vec![];

        if !dir.as_ref().exists() {
            return Ok(vec![]);
        }

        let contents = fs::read_dir(dir.as_ref())?;

        for entry in contents {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();

            if file_type.is_dir() {
                dirs.push(path);
            } else if file_type.is_file() {
                files.push(path);
            }
        }

        let nested_files = dirs
            .par_iter()
            .map(recurse)
            .try_reduce(Vec::new, |acc, files| Ok(acc.into_iter().chain(files).collect()))?;

        Ok(files.into_iter().chain(nested_files).collect())
    }

    recurse(root)
}

/// Remove all empty folders from `starting` and moving up until `root`
///
/// `root` must be a prefix / ancestor of `starting`
fn remove_empty_dirs(starting: &Path, root: &Path) -> io::Result<()> {
    if !starting.starts_with(root) || !starting.is_dir() || !root.is_dir() {
        return Ok(());
    }

    let mut current = Some(starting);

    while let Some(dir) = current.take() {
        if dir.exists() {
            let is_empty = fs::read_dir(dir)?.count() == 0;

            if !is_empty {
                return Ok(());
            }

            fs::remove_dir(dir)?;
        }

        if let Some(parent) = dir.parent()
            && parent != root
        {
            current = Some(parent);
        }
    }

    Ok(())
}

fn remove_archived_states(archive_paths: &[PathBuf]) -> Result<(), Error> {
    println!();

    let progressbar = ProgressBar::new(archive_paths.len() as u64).with_style(
        ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
            .unwrap()
            .progress_chars("■= "),
    );
    progressbar.tick();

    let counter = Arc::new(AtomicUsize::new(0));

    util::par_remove_dirs_all(archive_paths, |path, res| match res {
        Ok(_) => {
            let cnt = counter.fetch_add(1, Ordering::Relaxed) + 1;
            info!(
                progress = cnt as f32 / archive_paths.len() as f32,
                current = cnt,
                total = archive_paths.len(),
                event_type = "progress_update",
                "Removed archived state: {:?}",
                path
            );
            progressbar.inc(1);
            progressbar.suspend(|| println!("{} {path:?}", "Removed".green()));
        }
        Err(e) => eprintln!("Failed to remove archived state: {path:?} ({e})"),
    })?;

    progressbar.finish_and_clear();

    Ok(())
}

/// Simple timing information for Prune
#[derive(Default)]
pub struct Timing {
    pub resolve: Duration,
    pub prune_db: Duration,
    pub orphaned_files: Duration,
    pub prune_archives: Duration,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("no active state found")]
    NoActiveState,
    #[error("cannot prune the currently active state")]
    PruneCurrent,
    #[error("db")]
    DB(#[from] db::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),
    #[error("synchronize boot")]
    SyncBoot(#[source] boot::Error),
}
