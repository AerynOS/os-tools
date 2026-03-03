// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{fs, io, path::Path, time::Duration};

use futures_util::{StreamExt, TryStreamExt, stream};
use moss::{runtime, util};
use nix::unistd::{LinkatFlags, linkat};
use stone_recipe::upstream::{self, SourceUri};
use thiserror::Error;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::{
    Paths, Recipe,
    upstream::{
        git::{Git, StoredGit},
        plain::{Plain, StoredPlain},
    },
};

pub mod git;
pub mod plain;

#[derive(Debug, Clone)]
pub enum Upstream {
    Plain(Plain),
    Git(Git),
}

impl Upstream {
    pub fn from_recipe(upstream: upstream::Upstream, original_index: usize) -> Result<Self, Error> {
        match upstream.props {
            upstream::Props::Plain { hash, rename, .. } => Ok(Self::Plain(Plain {
                url: upstream.url,
                hash: hash.parse().map_err(|e| plain::Error::from(e))?,
                rename,
            })),
            upstream::Props::Git { git_ref, staging } => Ok(Self::Git(Git {
                uri: upstream.url,
                ref_id: git_ref,
                staging,
                original_index,
            })),
        }
    }

    pub async fn fetch_new(uri: SourceUri, dest: &Path) -> Result<Self, Error> {
        Ok(match uri.kind {
            upstream::Kind::Archive => Self::Plain(Plain::fetch_new(uri.url, &dest).await?),
            upstream::Kind::Git => Self::Git(Git::fetch_new(&uri.url, &dest).await?),
        })
    }

    fn name(&self) -> &str {
        match self {
            Upstream::Plain(plain) => plain.name(),
            Upstream::Git(git) => git.name(),
        }
    }

    async fn store(&self, paths: &Paths, pb: &ProgressBar) -> Result<Stored, Error> {
        Ok(match self {
            Upstream::Plain(plain) => Stored::Plain(plain.store(paths, pb).await?),
            Upstream::Git(git) => Stored::Git(git.store(paths, pb).await?),
        })
    }

    fn remove(&self, paths: &Paths) -> Result<(), Error> {
        Ok(match self {
            Upstream::Plain(plain) => plain.remove(paths)?,
            Upstream::Git(git) => git.remove(paths)?,
        })
    }
}

#[derive(Clone)]
pub(crate) enum Stored {
    Plain(StoredPlain),
    Git(StoredGit),
}

impl Stored {
    fn was_cached(&self) -> bool {
        match self {
            Stored::Plain(plain) => plain.was_cached,
            Stored::Git(git) => git.was_cached,
        }
    }

    fn share(&self, dest_dir: &Path) -> Result<(), Error> {
        match self {
            Stored::Plain(plain) => {
                let target = dest_dir.join(plain.name.clone());

                // Attempt hard link
                let link_result = linkat(None, &plain.path, None, &target, LinkatFlags::NoSymlinkFollow);

                // Copy instead
                if link_result.is_err() {
                    fs::copy(plain.path.clone(), &target)?;
                }
            }
            Stored::Git(git) => {
                let target = dest_dir.join(git.name.clone());
                util::copy_dir(&git.path, &target)?;
            }
        }

        Ok(())
    }
}

pub fn parse(recipe: &Recipe) -> Result<Vec<Upstream>, Error> {
    recipe
        .parsed
        .upstreams
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, upstream)| Upstream::from_recipe(upstream, index))
        .collect()
}

/// Cache all upstreams from the provided [`Recipe`], make them available
/// in the guest rootfs, and update the stone.yaml with resolved git upstream hashes.
pub fn sync(recipe: &Recipe, paths: &Paths, upstreams: &[Upstream]) -> Result<(), Error> {
    println!();
    println!("Sharing {} upstream(s) with the build container", upstreams.len());

    let mp = MultiProgress::new();
    let tp = mp.add(
        ProgressBar::new(upstreams.len() as u64).with_style(
            ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
                .unwrap()
                .progress_chars("■≡=- "),
        ),
    );
    tp.tick();

    let upstream_dir = paths.guest_host_path(&paths.upstreams());
    util::ensure_dir_exists(&upstream_dir)?;

    let installed_upstreams = runtime::block_on(
        stream::iter(upstreams)
            .map(|upstream| async {
                let pb = mp.insert_before(
                    &tp,
                    ProgressBar::new(u64::MAX).with_message(format!(
                        "{} {}",
                        "Downloading".blue(),
                        upstream.name().bold(),
                    )),
                );
                pb.enable_steady_tick(Duration::from_millis(150));

                let install = upstream.store(paths, &pb).await?;

                pb.set_message(format!("{} {}", "Copying".yellow(), upstream.name().bold()));
                pb.set_style(
                    ProgressStyle::with_template(" {spinner} {wide_msg} ")
                        .unwrap()
                        .tick_chars("--=≡■≡=--"),
                );

                runtime::unblock({
                    let install = install.clone();
                    let dir = upstream_dir.clone();
                    move || install.share(&dir)
                })
                .await?;

                let cached_tag = install
                    .was_cached()
                    .then_some(format!("{}", " (cached)".dim()))
                    .unwrap_or_default();

                pb.finish();
                mp.remove(&pb);
                mp.suspend(|| println!("{} {}{cached_tag}", "Shared".green(), upstream.name().bold()));
                tp.inc(1);

                Ok(install) as Result<_, Error>
            })
            .buffer_unordered(moss::environment::MAX_NETWORK_CONCURRENCY)
            .try_collect::<Vec<_>>(),
    )?;

    if let Some(updated_yaml) = git::update_git_upstream_refs(&recipe.source, &installed_upstreams)? {
        fs::write(&recipe.path, updated_yaml)?;
        println!(
            "{} | Git references resolved to commit hashes and saved to stone.yaml. This ensures reproducible builds since tags and branches can move over time.",
            "Warning".yellow()
        );
    }

    mp.clear()?;
    println!();

    Ok(())
}

pub fn remove(paths: &Paths, upstreams: &[Upstream]) -> Result<(), Error> {
    for upstream in upstreams {
        upstream.remove(paths)?;
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("git")]
    Git(#[from] git::Error),
    // FIXME: this error comes from a module that
    // used to live on its own. Now it's merged into this one,
    // thus there is no need for duplicated error types.
    #[error("git")]
    GitOperation(#[from] git::GitError),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("plain")]
    Plain(#[from] plain::Error),
    #[error("request")]
    Request(#[from] moss::request::Error),
}
