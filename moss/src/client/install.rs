// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Installation-specific code for several core moss operations

use std::time::{Duration, Instant};

use thiserror::Error;
use tui::{
    dialoguer::{theme::ColorfulTheme, Confirm},
    pretty::autoprint_columns,
};

use crate::{
    client::{self, Client},
    package::{self, Flags},
    registry::transaction,
    runtime,
    state::Selection,
    Package, Provider,
};

/// Install a set of packages.
///
/// If this call is successful a new State is recorded into the [`super::db::state::Database`].
/// Upon completion the `/usr` tree is "hot swapped" with the staging tree through `renameat2` call.
pub fn install(client: &mut Client, pkgs: &[&str], yes: bool) -> Result<Timing, Error> {
    let mut timing = Timing::default();
    let mut instant = Instant::now();

    // Resolve input packages
    let input = resolve_input(pkgs, client)?;

    // Add all inputs
    let mut tx = client.registry.transaction()?;

    tx.add(input.clone())?;

    // Resolve transaction to metadata
    let resolved = client.resolve_packages(tx.finalize())?;

    // Get installed packages to check against
    let installed = client.registry.list_installed(Flags::default()).collect::<Vec<_>>();
    let is_installed = |p: &Package| installed.iter().any(|i| i.meta.name == p.meta.name);

    // Get missing packages that are:
    //
    // Stateful: Not installed
    // Ephemeral: all
    let missing = resolved
        .iter()
        .filter(|p| client.is_ephemeral() || !is_installed(p))
        .collect::<Vec<_>>();

    timing.resolve = instant.elapsed();

    // If no new packages exist, exit and print
    // packages already installed
    if missing.is_empty() {
        let installed = resolved
            .iter()
            .filter(|p| is_installed(p) && input.contains(&p.id))
            .collect::<Vec<_>>();

        if !installed.is_empty() {
            println!("The following package(s) are already installed:");
            println!();
            autoprint_columns(&installed);
        }

        return Ok(timing);
    }

    // Testing panic for hyperfine benchmarking purposes (build flag tuning)
    // panic!();

    println!("The following package(s) will be installed:");
    println!();
    autoprint_columns(&missing);
    println!();

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

    // Cache packages
    runtime::block_on(client.cache_packages(&missing))?;

    timing.fetch = instant.elapsed();
    instant = Instant::now();

    // Calculate the new state of packages (old_state + missing)
    let new_state_pkgs = {
        // Only use previous state in stateful mode
        let previous_selections = match client.installation.active_state {
            Some(id) if !client.is_ephemeral() => client.state_db.get(id)?.selections,
            _ => vec![],
        };
        let missing_selections = missing.iter().map(|p| Selection {
            package: p.id.clone(),
            // Package is explicit if it was one of the input
            // packages provided by the user
            explicit: input.iter().any(|id| *id == p.id),
            reason: None,
        });

        missing_selections.chain(previous_selections).collect::<Vec<_>>()
    };

    // Perfect, apply state.
    client.new_state(&new_state_pkgs, "Install")?;

    timing.blit = instant.elapsed();

    Ok(timing)
}

/// Resolves the package arguments as valid input packages. Returns an error
/// if any args are invalid.
fn resolve_input(pkgs: &[&str], client: &Client) -> Result<Vec<package::Id>, Error> {
    // Parse pkg args into valid / invalid sets
    let queried = pkgs.iter().map(|p| find_packages(p, client));

    let mut results = vec![];

    for (id, pkg) in queried {
        if let Some(pkg) = pkg {
            results.push(pkg.id);
        } else {
            return Err(Error::NoPackage(id));
        }
    }

    Ok(results)
}

/// Resolve a package name to the first package
fn find_packages(id: &str, client: &Client) -> (String, Option<Package>) {
    let provider = Provider::from_name(id).unwrap();
    let result = client
        .registry
        .by_provider(&provider, Flags::new().with_available())
        .next();

    // First only, pre-sorted
    (id.into(), result)
}

/// Simple timing information for Install
#[derive(Default)]
pub struct Timing {
    pub resolve: Duration,
    pub fetch: Duration,
    pub blit: Duration,
}

/// Error's specific to installation operations
#[derive(Debug, Error)]
pub enum Error {
    /// The operation was explicitly cancelled at the user's request
    #[error("cancelled")]
    Cancelled,

    /// An error originated in [`client`] module
    #[error("client")]
    Client(#[from] client::Error),

    /// The given package couldn't be found
    #[error("no package found: {0}")]
    NoPackage(String),

    /// A transaction specific error occurred
    #[error("transaction")]
    Transaction(#[from] transaction::Error),

    /// A database specific error occurred
    #[error("db")]
    DB(#[from] crate::db::Error),

    /// Had issues processing user-provided string input
    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),

    /// We forgot how disks work
    #[error("io")]
    Io(#[from] std::io::Error),
}
