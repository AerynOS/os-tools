// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    path::{Path, PathBuf},
};

use astr::AStr;
use fs_err as fs;
use rayon::iter::{IntoParallelIterator as _, ParallelBridge, ParallelIterator as _};
use stone::{StoneDigestWriter, StoneDigestWriterHasher, StonePayloadLayoutFile};
use tui::{
    MultiProgress, ProgressBar, ProgressStyle, Styled,
    dialoguer::{Confirm, theme::ColorfulTheme},
};
use vfs::tree::BlitFile;

use crate::{
    Client, Package, Signal,
    client::{self, cache},
    package, runtime, signal, state,
};

pub fn verify(client: &Client, yes: bool, verbose: bool) -> Result<(), client::Error> {
    println!("Verifying assets");

    // Get all installed layouts, this is our source of truth
    let layouts = client.layout_db.all()?;

    // Group by unique assets (hash)
    let mut unique_assets = BTreeMap::new();
    for (package, layout) in layouts {
        let StonePayloadLayoutFile::Regular(hash, file) = layout.file else {
            continue;
        };
        unique_assets
            .entry(format!("{hash:02x}"))
            .or_insert_with(Vec::new)
            .push((package, file));
    }

    let mpb = MultiProgress::new();
    let pb = mpb.add(
        ProgressBar::new(unique_assets.len() as u64)
            .with_message("Verifying")
            .with_style(
                ProgressStyle::with_template("\n|{bar:20.red/blue}| {pos}/{len} {wide_msg}")
                    .unwrap()
                    .progress_chars("■≡=- "),
            ),
    );
    pb.tick();

    // For each asset, ensure it exists in the content store and isn't corrupt (hash is correct)
    let mut issues = unique_assets
        .into_par_iter()
        .try_fold(Vec::new, |mut acc, (hash, meta)| -> io::Result<_> {
            // Padded so output is consistent
            let display_hash = format!("{hash:0>32}");

            let path = cache::asset_path(&client.installation, &hash);

            let files = meta.iter().map(|(_, file)| file).cloned().collect::<BTreeSet<_>>();

            pb.inc(1);
            pb.set_message(format!("Verifying {display_hash}"));

            if !path.exists() {
                if verbose {
                    mpb.suspend(|| println!(" {} {display_hash} - {files:?}", "×".yellow()));
                }
                acc.push(Issue::MissingCasAsset {
                    hash,
                    files,
                    packages: meta.into_iter().map(|(package, _)| package).collect(),
                });
                return Ok(acc);
            }

            let verified_hash = xxh3_128_hash(&path)?;

            if verified_hash != hash {
                if verbose {
                    mpb.suspend(|| println!(" {} {display_hash} - {files:?}", "×".yellow()));
                }
                acc.push(Issue::CorruptCasAsset {
                    hash,
                    files,
                    packages: meta.into_iter().map(|(package, _)| package).collect(),
                });
                return Ok(acc);
            }

            if verbose {
                mpb.suspend(|| println!(" {} {display_hash} - {files:?}", "»".green()));
            }

            Ok(acc)
        })
        .try_reduce(Vec::new, try_reduce_vec_concat)?;

    // Get all states
    let states = client.state_db.all()?;

    pb.set_length(states.len() as u64);
    pb.set_position(0);
    pb.set_message("");

    mpb.suspend(|| {
        println!("Verifying states");
    });

    let vfs_pb = mpb.add(
        ProgressBar::new(0).with_style(
            ProgressStyle::with_template("|{bar:20.red/blue}| {pos}/{len} {wide_msg}")
                .unwrap()
                .progress_chars("■≡=- "),
        ),
    );

    // Check the VFS of each state exists properly on the FS
    let states_issues = states.iter().try_fold(Vec::new(), |mut acc, state| {
        pb.inc(1);
        pb.set_message(format!("Verifying state #{}", state.id));
        vfs_pb.set_message("Calculating...");
        vfs_pb.set_position(0);
        vfs_pb.set_length(1);
        vfs_pb.force_draw();

        let is_active = client.installation.active_state == Some(state.id);

        let base = if is_active {
            client.installation.root.join("usr")
        } else {
            client.installation.root_path(state.id.to_string()).join("usr")
        };

        let vfs = client.vfs(state.selections.iter().map(|s| &s.package))?;

        vfs_pb.set_length(vfs.len());

        let state_issues: Vec<_> = vfs
            .iter()
            .par_bridge()
            .filter_map(|file| {
                vfs_pb.inc(1);
                vfs_pb.set_message(format!("{}", file.path()));

                let hash = if let StonePayloadLayoutFile::Regular(hash, _) = file.layout.file {
                    Some(format!("{hash:02x}"))
                } else {
                    None
                };

                let path = base.join(file.path().strip_prefix("/usr/").unwrap_or_default());

                // All symlinks for non-active states are broken
                // since they resolve to the active state path
                //
                // Use try_exists to ensure we only check if symlink
                // itself is missing
                match path.try_exists() {
                    Ok(true) => {
                        // Validate regular file hash per state.
                        // This can be different from the backing
                        // CAS asset due to reflink / CoW.
                        if let Some(hash) = hash {
                            let verified_hash = match xxh3_128_hash(&path) {
                                Ok(verified) => verified,
                                Err(err) => return Some(Err(err)),
                            };

                            if verified_hash != hash {
                                return Some(Ok(Issue::CorruptStateAsset { path, state: state.id }));
                            }
                        }

                        None
                    }
                    Ok(false) if path.is_symlink() => None,
                    _ => Some(Ok(Issue::MissingStateAsset { path, state: state.id })),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        if verbose {
            let mark = if !state_issues.is_empty() {
                "×".yellow()
            } else {
                "»".green()
            };
            mpb.suspend(|| println!(" {mark} state #{}", state.id));
        }

        acc.extend(state_issues);
        Ok::<_, super::Error>(acc)
    })?;
    issues.extend(states_issues);

    vfs_pb.finish_and_clear();
    pb.finish_and_clear();
    mpb.remove(&vfs_pb);
    mpb.remove(&pb);

    if issues.is_empty() {
        println!("No issues found");
        return Ok(());
    }

    println!(
        "Found {} issue{}",
        issues.len(),
        if issues.len() == 1 { "" } else { "s" }
    );

    for issue in &issues {
        println!(" {} {issue}", "×".yellow());
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
        for corrupt_hash in issues.iter().filter_map(Issue::corrupt_cas_hash) {
            let path = cache::asset_path(&client.installation, corrupt_hash);
            fs::remove_file(&path)?;
        }

        println!("Reinstalling packages");

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

    println!("Reblitting affected states");

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
            let system_model =
                client.load_or_create_system_model(client.installation.root.join("usr/lib/system-model.kdl"), state)?;

            // Override install root with the newly blitted active state
            client.apply_stateful_blit(fstree, state, None, system_model)?;
            // Remove corrupt (swapped) state from staging directory
            fs::remove_dir_all(client.installation.staging_dir())?;
        } else {
            let system_model = client.load_or_create_system_model(
                client
                    .installation
                    .root_path(state.id.to_string())
                    .join("usr/lib/system-model.kdl"),
                state,
            )?;

            // Use the staged blit as an ephereral target for the non-active state
            // then archive it to it's archive directory
            client::record_state_id(&client.installation.staging_dir(), state.id)?;
            client.apply_ephemeral_blit(fstree, &client.installation.staging_dir(), system_model)?;

            // Remove the old archive state so the new blit can be archived
            fs::remove_dir_all(client.installation.root_path(state.id.to_string())).or_else(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
            client.archive_state(state.id)?;
        }

        println!(" {} state #{}", "»".green(), state.id);
    }

    println!("All issues resolved");

    Ok(())
}

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
enum Issue {
    CorruptCasAsset {
        hash: String,
        files: BTreeSet<AStr>,
        packages: BTreeSet<package::Id>,
    },
    MissingCasAsset {
        hash: String,
        files: BTreeSet<AStr>,
        packages: BTreeSet<package::Id>,
    },
    CorruptStateAsset {
        path: PathBuf,
        state: state::Id,
    },
    MissingStateAsset {
        path: PathBuf,
        state: state::Id,
    },
}

impl Issue {
    fn corrupt_cas_hash(&self) -> Option<&str> {
        match self {
            Issue::CorruptCasAsset { hash, .. } => Some(hash),
            Issue::MissingCasAsset { .. } | Issue::CorruptStateAsset { .. } | Issue::MissingStateAsset { .. } => None,
        }
    }

    fn packages(&self) -> Option<&BTreeSet<package::Id>> {
        match self {
            Issue::CorruptCasAsset { packages, .. } | Issue::MissingCasAsset { packages, .. } => Some(packages),
            Issue::CorruptStateAsset { .. } | Issue::MissingStateAsset { .. } => None,
        }
    }

    fn state(&self) -> Option<&state::Id> {
        match self {
            Issue::CorruptCasAsset { .. } | Issue::MissingCasAsset { .. } => None,
            Issue::CorruptStateAsset { state, .. } | Issue::MissingStateAsset { state, .. } => Some(state),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::CorruptCasAsset { hash, files, .. } => write!(f, "Corrupt asset {hash} - {files:?}"),
            Issue::MissingCasAsset { hash, files, .. } => write!(f, "Missing asset {hash} - {files:?}"),
            Issue::CorruptStateAsset { path, state } => write!(f, "Corrupt path {} in state #{state}", path.display()),
            Issue::MissingStateAsset { path, state } => write!(f, "Missing path {} in state #{state}", path.display()),
        }
    }
}

fn try_reduce_vec_concat<T, E>(mut a: Vec<T>, mut b: Vec<T>) -> Result<Vec<T>, E> {
    a.append(&mut b);
    Ok(a)
}

fn xxh3_128_hash(path: &Path) -> io::Result<String> {
    let mut hasher = StoneDigestWriterHasher::new();
    let mut digest_writer = StoneDigestWriter::new(io::sink(), &mut hasher);
    let mut file = fs::File::open(path)?;

    // Copy bytes to null sink so we don't
    // explode memory
    io::copy(&mut file, &mut digest_writer)?;

    Ok(format!("{:02x}", hasher.digest128()))
}
