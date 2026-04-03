// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::Path, time::Duration};

use crate::recipe::Recipe;
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::runtime;
use stone_recipe::upstream;
use thiserror::Error;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::upstream::{
    git::{Git, SharedGit, StoredGit},
    plain::{Plain, SharedPlain, StoredPlain},
};

mod git;
mod plain;

/// An upstream is a backend where
/// to get source code from.
#[derive(Debug, Clone)]
pub enum Upstream {
    /// An archive containing source code, typically
    /// a tarball. In order to be usable, it must compatible with
    /// [bsdtar](https://man.freebsd.org/cgi/man.cgi?query=bsdtar&sektion=1&format=html).
    Plain(Plain),
    /// The source code is from a Git repository.
    Git(Git),
}

impl Upstream {
    /// Constructs an [Upstream] based on the information provided
    /// in the `upstream` section of a Stone recipe.
    pub fn from_recipe_upstream(upstream: upstream::Upstream) -> Result<Self, Error> {
        match upstream.props {
            upstream::Props::Plain { hash, rename, .. } => Ok(Self::Plain(Plain {
                url: upstream.url,
                hash: hash.parse().map_err(plain::Error::from)?,
                rename,
            })),
            upstream::Props::Git { git_ref, .. } => Ok(Self::Git(Git {
                url: upstream.url,
                commit: git_ref,
            })),
        }
    }

    /// Returns the name of the upstream. This is an informal
    /// name used for logging or when a path to be created
    /// doesn't need to be unique.
    fn name(&self) -> &str {
        match self {
            Upstream::Plain(plain) => plain.name(),
            Upstream::Git(git) => git.name(),
        }
    }

    /// Stores the upstream into the storage directory.
    /// The final path contained in the storage directory, and the write logic,
    /// depend on the upstream variant. The final path where the upstream is stored
    /// is unique inside the storage directory.
    async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<Stored, Error> {
        Ok(match self {
            Upstream::Plain(plain) => Stored::Plain(plain.store(storage_dir, pb).await?),
            Upstream::Git(git) => Stored::Git(git.store(storage_dir, pb).await?),
        })
    }

    /// Unconditionally removes this Upstream's resources within the storage directory.
    /// If the resources do not exist, this function returns successfully
    /// (it is idempotent).
    ///
    /// Careful: this function does not validate the content!
    /// It will be removed even if it does not belong to this Upstream.
    fn remove(&self, storage_dir: &Path) -> Result<(), Error> {
        match self {
            Upstream::Plain(plain) => plain.remove(storage_dir).map_err(Error::from),
            Upstream::Git(git) => git.remove(storage_dir).map_err(Error::from),
        }
    }
}

/// Information available after [Upstream] is stored on disk.
pub(crate) enum Stored {
    Plain(StoredPlain),
    Git(StoredGit),
}

impl Stored {
    /// Whether the upstream did not need to be written at the moment
    /// of being stored, because the constant was already there and valid.
    fn was_cached(&self) -> bool {
        match self {
            Stored::Plain(plain) => plain.was_cached,
            Stored::Git(git) => git.was_cached,
        }
    }

    /// Shares the upstream in preparation of a build.
    ///
    /// This function tries to be as efficient as possible in terms
    /// of actual bytes written/copied, by linking files from the storage directory.
    async fn share(&self, dest_dir: &Path) -> Result<Shared, Error> {
        Ok(match self {
            Stored::Plain(plain) => Shared::Plain(plain.share(dest_dir)?),
            Stored::Git(git) => Shared::Git(git.share(&dest_dir.join(&git.name)).await?),
        })
    }
}

/// A shared upstream is a copy of an upstream
/// in a location useful for a build.
pub enum Shared {
    Plain(SharedPlain),
    Git(SharedGit),
}

impl Shared {
    /// Removes the shared upstream.
    /// Should the upstream no longer exist,
    /// this function returns successfully (it is idempotent).
    pub fn remove(&self) -> Result<(), Error> {
        match self {
            Self::Plain(plain) => plain.remove()?,
            Self::Git(git) => git.remove()?,
        };
        Ok(())
    }
}

/// Returns a list of upstream from a Stone recipe.
pub fn parse_recipe(recipe: &Recipe) -> Result<Vec<Upstream>, Error> {
    recipe
        .parsed
        .upstreams
        .iter()
        .cloned()
        .map(Upstream::from_recipe_upstream)
        .collect()
}

/// Helper that stores and shares a list of [Upstream]s.
pub fn sync(upstreams: &[Upstream], storage_dir: &Path, share_dir: &Path) -> Result<Vec<Shared>, Error> {
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

    let shared = runtime::block_on(
        stream::iter(upstreams)
            .map(async |upstream| -> Result<Shared, Error> {
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

                let shared = stored.share(share_dir).await?;

                let cached_tag = stored
                    .was_cached()
                    .then_some(format!("{}", " (cached)".dim()))
                    .unwrap_or_default();

                pb.finish();
                mp.remove(&pb);
                mp.suspend(|| println!("{} {}{cached_tag}", "Shared".green(), upstream.name().bold()));
                tp.inc(1);

                Ok(shared)
            })
            .buffer_unordered(moss::environment::MAX_NETWORK_CONCURRENCY)
            .try_collect(),
    )?;

    mp.clear()?;
    println!();

    Ok(shared)
}

/// Helper that removes a list of [Upstream]s from the storage directory.
pub fn remove(storage_dir: &Path, upstreams: &[Upstream]) -> Result<(), Error> {
    for upstream in upstreams {
        upstream.remove(storage_dir)?;
    }
    Ok(())
}

/// Possible errors returned by functions in this module.
#[derive(Debug, Error)]
pub enum Error {
    #[error("plain")]
    /// An error occurred while dealing with an archive-based [Upstream].
    Plain(#[from] plain::Error),
    /// An error occurred while dealing with a Git-based [Upstream].
    #[error("git")]
    Git(#[from] git::Error),
    #[error("io")]
    // A generic I/O error occurred.
    Io(#[from] io::Error),
}
