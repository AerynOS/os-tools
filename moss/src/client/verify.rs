// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, fmt, io, path::PathBuf};

use itertools::Itertools;

use fs_err as fs;
use stone::{payload::layout, write::digest};
use tui::{
    dialoguer::{theme::ColorfulTheme, Confirm},
    ProgressBar, ProgressStyle, Styled,
};
use vfs::tree::BlitFile;

use crate::{
    client::{self, cache},
    package, runtime, signal, state, Client, Package, Signal,
};

pub fn verify(client: &Client, yes: bool, verbose: bool) -> Result<(), client::Error> {
    tracing::info!("Verifying assets");

    // Get all installed layouts, this is our source of truth
    let layouts = client.layout_db.all()?;

    // Group by unique assets (hash)
    let unique_assets = layouts
        .into_iter()
        .filter_map(|(package, layout)| {
            if let layout::Entry::Regular(hash, file) = layout.entry {
                Some((format!("{hash:02x}"), (package, file)))
            } else {
                None
            }
        })
        .into_group_map();

    let mut issues = vec![];
    let mut hasher = digest::Hasher::new();

    let pb = ProgressBar::new(unique_assets.len() as u64)
        .with_message("Verifying")
        .with_style(
            ProgressStyle::with_template("\n|{bar:20.red/blue}| {pos}/{len} {wide_msg}")
                .unwrap()
                .progress_chars("■≡=- "),
        );
    pb.tick();

    // For each asset, ensure it exists in the content store and isn't corrupt (hash is correct)
    for (hash, meta) in unique_assets
        .into_iter()
        .sorted_by_key(|(key, _)| format!("{key:0>32}"))
    {
        let display_hash = format!("{hash:0>32}");

        let path = cache::asset_path(&client.installation, &hash);

        let files = meta.iter().map(|(_, file)| file).cloned().collect::<BTreeSet<_>>();

        pb.set_message(format!("Verifying {display_hash}"));

        if !path.exists() {
            pb.inc(1);
            if verbose {
                pb.suspend(|| tracing::warn!(" {} {display_hash} - {files:?}", "×".yellow()));
            }
            issues.push(Issue::MissingAsset {
                hash: display_hash,
                files,
                packages: meta.into_iter().map(|(package, _)| package).collect(),
            });
            continue;
        }

        hasher.reset();

        let mut digest_writer = digest::Writer::new(io::sink(), &mut hasher);
        let mut file = fs::File::open(&path)?;

        // Copy bytes to null sink so we don't
        // explode memory
        io::copy(&mut file, &mut digest_writer)?;

        let verified_hash = format!("{:02x}", hasher.digest128());

        if verified_hash != hash {
            pb.inc(1);
            if verbose {
                pb.suspend(|| tracing::warn!(" {} {display_hash} - {files:?}", "×".yellow()));
            }
            issues.push(Issue::CorruptAsset {
                hash: display_hash,
                files,
                packages: meta.into_iter().map(|(package, _)| package).collect(),
            });
            continue;
        }

        pb.inc(1);
        if verbose {
            pb.suspend(|| tracing::warn!(" {} {display_hash} - {files:?}", "»".green()));
        }
    }

    // Get all states
    let states = client.state_db.all()?;

    pb.set_length(states.len() as u64);
    pb.set_position(0);
    pb.suspend(|| {
        tracing::info!("Verifying states");
    });

    // Check the VFS of each state exists properly on the FS
    for state in &states {
        pb.set_message(format!("Verifying state #{}", state.id));

        let is_active = client.installation.active_state == Some(state.id);

        let vfs = client.vfs(state.selections.iter().map(|s| &s.package))?;

        let base = if is_active {
            client.installation.root.join("usr")
        } else {
            client.installation.root_path(state.id.to_string()).join("usr")
        };

        let files = vfs.iter().collect::<Vec<_>>();

        let mut num_issues = 0;

        for file in files {
            let path = base.join(file.path().strip_prefix("/usr/").unwrap_or_default());

            // All symlinks for non-active states are broken
            // since they resolve to the active state path
            //
            // Use try_exists to ensure we only check if symlink
            // itself is missing
            match path.try_exists() {
                Ok(true) => {}
                Ok(false) if path.is_symlink() => {}
                _ => {
                    num_issues += 1;
                    issues.push(Issue::MissingVFSPath { path, state: state.id });
                }
            }
        }

        pb.inc(1);
        if verbose {
            let mark = if num_issues > 0 { "×".yellow() } else { "»".green() };
            pb.suspend(|| tracing::warn!(" {mark} state #{}", state.id));
        }
    }

    pb.finish_and_clear();

    if issues.is_empty() {
        tracing::info!("No issues found");
        return Ok(());
    }

    tracing::warn!(
        "Found {} issue{}",
        issues.len(),
        if issues.len() == 1 { "" } else { "s" }
    );

    for issue in &issues {
        tracing::info!(" {} {issue}", "×".yellow());
    }

    let result = if yes {
        true
    } else {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(" Fixing issues, this will change your system state. Do you wish to continue? ")
            .default(false)
            .interact()?
    };
    if !result {
        return Err(client::Error::Cancelled);
    }

    // Calculate and resolve the unique set of packages with asset issues
    let issue_packages = issues
        .iter()
        .filter_map(Issue::packages)
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| {
            client.install_db.get(id).map(|meta| Package {
                id: id.clone(),
                meta,
                flags: package::Flags::default(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // We had some corrupt or missing assets, let's resolve that!
    if !issue_packages.is_empty() {
        // Remove all corrupt assets
        for corrupt_hash in issues.iter().filter_map(Issue::corrupt_hash) {
            let path = cache::asset_path(&client.installation, corrupt_hash);
            fs::remove_file(&path)?;
        }

        tracing::info!("Reinstalling packages");

        // And re-cache all packages that comprise the corrupt / missing asset
        runtime::block_on(client.cache_packages(&issue_packages))?;
    }

    // Now we must fix any states that referenced these packages
    // or had their own VFS issues that require a reblit
    let issue_states = states
        .iter()
        .filter_map(|state| {
            state
                .selections
                .iter()
                .any(|s| issue_packages.iter().any(|p| p.id == s.package))
                .then_some(&state.id)
        })
        .chain(issues.iter().filter_map(Issue::state))
        .collect::<BTreeSet<_>>();

    tracing::info!("Reblitting affected states");

    let _guard = signal::ignore([Signal::SIGINT])?;
    let _fd = signal::inhibit(
        vec!["shutdown", "sleep", "idle", "handle-lid-switch"],
        "moss".into(),
        "Verifying states".into(),
        "block".into(),
    );

    // Reblit each state
    for id in issue_states {
        let state = states
            .iter()
            .find(|s| s.id == *id)
            .expect("must come from states originally");

        let is_active = client.installation.active_state == Some(state.id);

        // Blits to staging dir
        let fstree = client.blit_root(state.selections.iter().map(|s| &s.package))?;

        if is_active {
            // Override install root with the newly blitted active state
            client.apply_stateful_blit(fstree, state, None)?;
            // Remove corrupt (swapped) state from staging directory
            fs::remove_dir_all(client.installation.staging_dir())?;
        } else {
            // Use the staged blit as an ephereral target for the non-active state
            // then archive it to it's archive directory
            client::record_state_id(&client.installation.staging_dir(), state.id)?;
            client.apply_ephemeral_blit(fstree, &client.installation.staging_dir())?;

            // Remove the old archive state so the new blit can be archived
            fs::remove_dir_all(client.installation.root_path(state.id.to_string()))?;
            client.archive_state(state.id)?;
        }

        tracing::info!(" {} state #{}", "»".green(), state.id);
    }

    tracing::info!("All issues resolved");

    Ok(())
}

#[derive(Debug)]
enum Issue {
    CorruptAsset {
        hash: String,
        files: BTreeSet<String>,
        packages: BTreeSet<package::Id>,
    },
    MissingAsset {
        hash: String,
        files: BTreeSet<String>,
        packages: BTreeSet<package::Id>,
    },
    MissingVFSPath {
        path: PathBuf,
        state: state::Id,
    },
}

impl Issue {
    fn corrupt_hash(&self) -> Option<&str> {
        match self {
            Issue::CorruptAsset { hash, .. } => Some(hash),
            Issue::MissingAsset { .. } => None,
            Issue::MissingVFSPath { .. } => None,
        }
    }

    fn packages(&self) -> Option<&BTreeSet<package::Id>> {
        match self {
            Issue::CorruptAsset { packages, .. } | Issue::MissingAsset { packages, .. } => Some(packages),
            Issue::MissingVFSPath { .. } => None,
        }
    }

    fn state(&self) -> Option<&state::Id> {
        match self {
            Issue::CorruptAsset { .. } | Issue::MissingAsset { .. } => None,
            Issue::MissingVFSPath { state, .. } => Some(state),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::CorruptAsset { hash, files, .. } => write!(f, "Corrupt asset {hash} - {files:?}"),
            Issue::MissingAsset { hash, files, .. } => write!(f, "Missing asset {hash} - {files:?}"),
            Issue::MissingVFSPath { path, state } => write!(f, "Missing path {} in state #{state}", path.display()),
        }
    }
}
