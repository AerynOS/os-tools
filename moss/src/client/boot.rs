// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
// SPDX-FileCopyrightText: Copyright © 2025 AerynOS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Boot management integration in moss

use std::{
    io,
    path::{Path, PathBuf},
    str::FromStr,
    vec,
};

use blsforme::{
    CmdlineEntry, Entry, Schema,
    os_release::{self, OsRelease},
};
use fnmatch::Pattern;
use fs_err as fs;
use itertools::Itertools;
use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};
use thiserror::{self, Error};

use crate::{Installation, State, db, package::Id};

use super::Client;

#[derive(Debug, Error)]
pub enum Error {
    #[error("blsforme: {0}")]
    Blsforme(#[from] blsforme::Error),

    #[error("layoutdb: {0}")]
    Client(#[from] db::layout::Error),

    #[error("io: {0}")]
    IO(#[from] io::Error),

    #[error("os_info: {0}")]
    OsInfo(#[from] os_info::Error),

    #[error("os_release: {0}")]
    OsRelease(#[from] os_release::Error),

    /// fnmatch pattern compilation for boot, etc.
    #[error("fnmatch pattern: {0}")]
    Pattern(#[from] fnmatch::Error),

    #[error("incomplete kernel tree: {0}")]
    IncompleteKernel(String),
}

/// Simple mapping type for kernel discovery paths, retaining the layout reference
#[derive(Debug)]
struct KernelCandidate {
    path: PathBuf,
    _layout: StonePayloadLayoutRecord,
}

impl AsRef<Path> for KernelCandidate {
    fn as_ref(&self) -> &Path {
        self.path.as_path()
    }
}

/// From a given set of input paths, produce a set of match pairs
/// This is applied against the given system root
fn kernel_files_from_state<'a>(
    layouts: &'a [(Id, StonePayloadLayoutRecord)],
    pattern: &'a Pattern,
) -> Vec<KernelCandidate> {
    let mut kernel_entries = vec![];

    for (_, path) in layouts.iter() {
        match &path.file {
            StonePayloadLayoutFile::Regular(_, target) => {
                if pattern.match_path(target).is_some() {
                    kernel_entries.push(KernelCandidate {
                        path: PathBuf::from("usr").join(target),
                        _layout: path.to_owned(),
                    });
                }
            }
            StonePayloadLayoutFile::Symlink(_, target) => {
                if pattern.match_path(target).is_some() {
                    kernel_entries.push(KernelCandidate {
                        path: PathBuf::from("usr").join(target),
                        _layout: path.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }

    kernel_entries
}

/// Find bootloader assets in the new state
fn boot_files_from_new_state<'a>(
    install: &Installation,
    layouts: &'a [(Id, StonePayloadLayoutRecord)],
    pattern: &'a Pattern,
) -> Vec<PathBuf> {
    let mut rets = vec![];

    for (_, path) in layouts.iter() {
        if let StonePayloadLayoutFile::Regular(_, target) = &path.file
            && pattern.match_path(target).is_some()
        {
            rets.push(install.root.join("usr").join(target));
        }
    }

    rets
}

/// Grab all layouts for the provided state, mapped to package id
fn layouts_for_state(client: &Client, state: &State) -> Result<Vec<(Id, StonePayloadLayoutRecord)>, db::Error> {
    client.layout_db.query(state.selections.iter().map(|s| &s.package))
}

/// Return an additional 4 older states excluding the current state
fn states_except_new(client: &Client, state: &State) -> Result<Vec<State>, db::Error> {
    let states = client
        .state_db
        .list_ids()?
        .into_iter()
        .filter_map(|(id, whence)| {
            // All states with older ID and not the current state
            if id != state.id && state.id > id {
                Some((id, whence))
            } else {
                None
            }
        })
        .sorted_by_key(|(_, whence)| whence.to_owned())
        .rev()
        .take(4)
        .filter_map(|(id, _)| client.state_db.get(id).ok())
        .collect::<Vec<_>>();
    Ok(states)
}

/// Generate a schema for the root
fn os_schema_for_root(root: &Path) -> Result<Schema, Error> {
    let os_info_path = root.join("usr").join("lib").join("os-info.json");
    let os_release_path = root.join("usr").join("lib").join("os-release");

    if os_info_path.exists() {
        let info = os_info::load_os_info_from_path(&os_info_path)?;
        Ok(Schema::OsInfo {
            os_info: Box::new(info),
        })
    } else {
        let os_release = fs::read_to_string(os_release_path)?;
        let os_release = OsRelease::from_str(&os_release)?;
        Ok(Schema::Blsforme {
            os_release: Box::new(os_release),
        })
    }
}

pub fn synchronize(client: &Client, state: &State) -> Result<(), Error> {
    let root = client.installation.root.clone();
    let is_native = root.to_string_lossy() == "/";
    // Create an appropriate configuration
    let config = blsforme::Configuration {
        root: if is_native {
            blsforme::Root::Native(root.clone())
        } else {
            blsforme::Root::Image(root.clone())
        },
        vfs: "/".into(),
    };

    // For the new/active state
    let head_layouts = layouts_for_state(client, state)?;
    let kernel_pattern = Pattern::from_str("lib/kernel/(version:*)/*")?;
    let systemd = Pattern::from_str("lib*/systemd/boot/efi/*.efi")?;
    let booty_bits = boot_files_from_new_state(&client.installation, &head_layouts, &systemd);

    let mut all_states = states_except_new(client, state)?;

    // no fun times without a bootloder
    if booty_bits.is_empty() {
        return Ok(());
    }

    let global_schema = os_schema_for_root(&root)?;

    // Grab the entries for the new state
    let mut all_kernels = vec![];
    all_states.insert(0, state.clone());
    for state in all_states.iter() {
        let layouts = layouts_for_state(client, state)?;
        let local_kernels = kernel_files_from_state(&layouts, &kernel_pattern);
        let mapped = global_schema.discover_system_kernels(local_kernels.into_iter())?;
        all_kernels.push((mapped, state.id));
    }

    // pipe all of our entries into blsforme
    let mut entries = all_kernels
        .iter()
        .flat_map(|(kernels, state_id)| {
            kernels
                .iter()
                .filter_map(|k| {
                    let sysroot = if state.id == *state_id {
                        root.clone()
                    } else {
                        client.installation.root_path(state_id.to_string()).to_owned()
                    };

                    if !sysroot.exists() {
                        return None;
                    }

                    let local_schema = os_schema_for_root(&sysroot).ok();
                    let entry = Entry::new(k)
                        .with_cmdline(CmdlineEntry {
                            name: "---fstx---".to_owned(),
                            snippet: format!("moss.fstx={state_id}"),
                        })
                        .with_state_id(i32::from(*state_id))
                        .with_sysroot(sysroot);

                    match local_schema {
                        Some(schema) => Some(entry.with_schema(schema)),
                        None => Some(entry),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for entry in entries.iter_mut() {
        if let Err(e) = entry.load_cmdline_snippets(&config) {
            log::warn!("Failed to load cmdline snippets: {e}");
        }
    }
    // no usable entries, lets get out of here.
    if entries.is_empty() {
        return Ok(());
    }

    // If we can't get a manager, find, but don't bomb. Its probably a topology failure.
    let manager = match blsforme::Manager::new(&config) {
        Ok(m) => m.with_entries(entries.into_iter()).with_bootloader_assets(booty_bits),
        Err(_) => return Ok(()),
    };

    // Only allow mounting pre-sync for a native run
    if is_native {
        let _mounts = manager.mount_partitions()?;
        manager.sync(&global_schema)?;
    } else {
        manager.sync(&global_schema)?;
    }

    Ok(())
}
