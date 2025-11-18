// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! The pruning system for moss states and assets
//!
//! Quite simply this is a strategy based garbage collector for unused/unwanted
//! system states (i.e. historical snapshots) that cleans up database entries
//! and assets on disk by way of refcounting.

use std::collections::{BTreeMap, BTreeSet};
use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err as fs;
use itertools::Itertools;
use thiserror::Error;

use tui::{
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::autoprint_columns,
};

use crate::{Installation, State, client::cache, db, package, state};

/// The prune strategy for removing old states
#[derive(Debug, Clone, Copy)]
pub enum Strategy {
    /// Keep the most recent N states, remove the rest
    KeepRecent { keep: u64, include_newer: bool },
    /// Removes a specific state
    Remove(state::Id),
}

/// Prune old states using [`Strategy`] and garbage collect
/// all cached data related to those states being removed
///
/// # Arguments
///
/// * - `strategy`     - pruning strategy to employ
/// * - `state_db`     - Installation's state database
/// * - `install_db`   - Installation's "installed" database
/// * - `layout_db`    - Installation's layout database
/// * - `installation` - Client specific target filesystem encapsulation
pub fn prune(
    strategy: Strategy,
    state_db: &db::state::Database,
    install_db: &db::meta::Database,
    layout_db: &db::layout::Database,
    installation: &Installation,
    yes: bool,
) -> Result<(), Error> {
    // Only prune if the moss root has an active state (otherwise
    // it's probably borked or not setup yet)
    let Some(current_state) = installation.active_state else {
        return Err(Error::NoActiveState);
    };

    let state_ids = state_db.list_ids()?;

    // Find each state we need to remove
    let removal_ids = match strategy {
        Strategy::KeepRecent { keep, include_newer } => {
            // Filter for all removal candidates
            let candidates = state_ids
                .iter()
                .filter(|(id, _)| {
                    if include_newer {
                        *id != current_state
                    } else {
                        *id < current_state
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
            // Remove if this id actually exists
            .find_map(|(id, _)| (*id == remove).then_some(remove))
            .into_iter()
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
            if id == current_state {
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

    // Remove each state's archive folder
    for state in removals {
        let archive_path = installation.root_path(state.id.to_string());

        if archive_path.exists() {
            fs::remove_dir_all(&archive_path)?;
        }
    }

    Ok(())
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
) -> Result<(), Error> {
    // Compute hashes to remove by (installed - final)
    let installed_hashes = enumerate_file_hashes(&root)?;
    let hashes_to_remove = installed_hashes.difference(&final_hashes);

    // Remove each and it's parent dir if empty
    hashes_to_remove.into_iter().try_for_each(|hash| {
        // Compute path to file using hash
        let Some(file) = compute_path(hash.clone()) else {
            return Ok(());
        };
        let partial = file.with_extension("part");

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

        Ok(()) as Result<(), Error>
    })?;

    Ok(())
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
}
