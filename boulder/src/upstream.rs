// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::Path, time::Duration};

use crate::recipe::Recipe;
use elf::abi::SHT_MIPS_REGINFO;
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::{runtime, util};
use stone_recipe::upstream::{self, SourceUri};
use thiserror::Error;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::upstream::{
    git::{Git, SharedGit, StoredGit},
    plain::{Plain, SharedPlain, StoredPlain},
};

mod git;
mod plain;

#[derive(Debug, Clone)]
pub enum Upstream {
    Plain(Plain),
    Git(Git),
}

impl Upstream {
    pub fn from_recipe(upstream: upstream::Upstream) -> Result<Self, Error> {
        match upstream.props {
            upstream::Props::Plain { hash, rename, .. } => Ok(Self::Plain(Plain {
                url: upstream.url,
                hash: hash.parse().map_err(plain::Error::from)?,
                rename,
            })),
            upstream::Props::Git { git_ref, .. } => Ok(Self::Git(Git {
                url: upstream.url,
                ref_id: git_ref,
            })),
        }
    }

    pub async fn fetch_new(uri: SourceUri, dest: &Path) -> Result<Self, Error> {
        Ok(match uri.kind {
            upstream::Kind::Archive => Self::Plain(Plain::fetch_new(uri.url, dest).await?),
            upstream::Kind::Git => Self::Git(Git::fetch_new(&uri.url, dest).await?),
        })
    }

    fn name(&self) -> &str {
        match self {
            Upstream::Plain(plain) => plain.name(),
            Upstream::Git(git) => git.name(),
        }
    }

    async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<Stored, Error> {
        Ok(match self {
            Upstream::Plain(plain) => Stored::Plain(plain.store(storage_dir, pb).await?),
            Upstream::Git(git) => Stored::Git(git.store(&storage_dir.join("git"), pb).await?),
        })
    }

    fn stored(&self, storage_dir: &Path) -> Result<Stored, Error> {
        Ok(match self {
            Upstream::Plain(plain) => Stored::Plain(plain.stored(storage_dir)?),
            Upstream::Git(git) => Stored::Git(git.stored(&storage_dir.join("git"))?.0),
        })
    }
}

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

    fn share(&self, dest_dir: &Path) -> Result<Shared, Error> {
        Ok(match self {
            Stored::Plain(plain) => Shared::Plain(plain.share(dest_dir)?),
            Stored::Git(git) => Shared::Git(git.share(dest_dir)?),
        })
    }

    fn remove(&self) -> Result<(), Error> {
        match self {
            Self::Plain(plain) => plain.remove()?,
            Self::Git(git) => git.remove()?,
        }
        Ok(())
    }
}

pub enum Shared {
    Plain(SharedPlain),
    Git(SharedGit),
}

impl Shared {
    pub fn remove(&self) -> Result<(), Error> {
        match self {
            Self::Plain(plain) => plain.remove()?,
            Self::Git(git) => git.remove()?,
        };
        Ok(())
    }
}

pub fn parse_recipe(recipe: &Recipe) -> Result<Vec<Upstream>, Error> {
    recipe
        .parsed
        .upstreams
        .iter()
        .cloned()
        .map(Upstream::from_recipe)
        .collect()
}

/// Cache all upstreams from the provided [`Recipe`], make them available
/// in the guest rootfs, and update the stone.yaml with resolved git upstream hashes.
pub fn sync(recipe: &Recipe, storage_dir: &Path, share_dir: &Path, upstreams: &[Upstream]) -> Result<(), Error> {
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

    runtime::block_on(
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

                let stored = upstream.store(storage_dir, &pb).await?;

                pb.set_message(format!("{} {}", "Copying".yellow(), upstream.name().bold()));
                pb.set_style(
                    ProgressStyle::with_template(" {spinner} {wide_msg} ")
                        .unwrap()
                        .tick_chars("--=≡■≡=--"),
                );

                stored.share(share_dir)?;

                let cached_tag = stored
                    .was_cached()
                    .then_some(format!("{}", " (cached)".dim()))
                    .unwrap_or_default();

                pb.finish();
                mp.remove(&pb);
                mp.suspend(|| println!("{} {}{cached_tag}", "Shared".green(), upstream.name().bold()));
                tp.inc(1);

                Ok(stored) as Result<_, Error>
            })
            .buffer_unordered(moss::environment::MAX_NETWORK_CONCURRENCY)
            .try_collect::<Vec<_>>(),
    )?;

    mp.clear()?;
    println!();

    Ok(())
}

pub fn remove(storage_dir: &Path, upstreams: &[Upstream]) -> Result<(), Error> {
    for upstream in upstreams {
        let stored = upstream.stored(storage_dir)?;
        stored.remove()?;
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("git")]
    Git(#[from] git::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("plain")]
    Plain(#[from] plain::Error),
    #[error("request")]
    Request(#[from] moss::request::Error),
}
