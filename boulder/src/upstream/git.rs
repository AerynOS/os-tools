// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err as fs;
use moss::util;
use thiserror::Error;
use tui::{ProgressBar, ProgressStyle};
use url::Url;

/// Upstream based on a Git repository.
#[derive(Clone, Debug)]
pub struct Git {
    /// URL of origin.
    pub url: Url,
    /// Hash of the commit to be considered as source.
    pub commit: String,
}

impl Git {
    /// Returns the name of the upstream. It is implied from the URL.
    pub fn name(&self) -> &str {
        util::uri_file_name(&self.url)
    }

    /// Stores the upstream into the storage directory.
    /// If the upstream was already stored but does not include [Self::commit],
    /// it is updated contextually. If it does not exist, the Git repository is cloned.
    pub async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<StoredGit, Error> {
        let repo: gitwrap::Repository;
        let mut cached = true;
        match self.stored(storage_dir).await {
            Ok((stored, has_commit)) => {
                repo = stored.repo;
                if !has_commit {
                    cached = false;
                    fetch(&repo, pb).await?;
                }
            }
            Err(Error::Git(_)) => {
                cached = false;
                self.remove(storage_dir)?;
                repo = clone(&self.url, &self.stored_path(storage_dir), pb).await?;
            }
            Err(Error::Io(e)) => return Err(Error::from(e)),
        }

        // When we reach this point, the repository may still
        // not have the commit ID (e.g. because the ID
        // was wrongly typed in the first place). This is acceptable
        // because we managed to store the repository nonetheless.
        // Users will receive an error when calling StoredGit::share.

        Ok(StoredGit {
            name: self.name().to_owned(),
            was_cached: cached,
            repo,
            commit: self.commit.to_owned(),
        })
    }

    /// Unconditionally removes the directory, within the storage
    /// directory, that would store the Git repository.
    /// If the directory does not exist, this function returns
    /// successfully (it is idempotent).
    ///
    /// Careful: this function does not validate the content
    /// of the directory! Resources will be deleted even if they
    /// do not belong to a Git repository.
    pub fn remove(&self, storage_dir: &Path) -> Result<(), Error> {
        let dir = self.stored_path(storage_dir);
        util::remove_dir_all(&dir).map_err(Error::from)
    }

    /// Returns the stored upstream if it exists.
    ///
    /// If successful, a tuple is returned containing the
    /// stored upstream and a boolean flag, indicating whether
    /// the stored Git repository contains [Self::commit].
    pub async fn stored(&self, storage_dir: &Path) -> Result<(StoredGit, bool), Error> {
        let repo = gitwrap::Repository::open_bare(&self.stored_path(storage_dir)).await?;
        let has_ref = repo.has_commit(&self.commit).await?;
        Ok((
            StoredGit {
                name: self.name().to_owned(),
                was_cached: has_ref,
                repo,
                commit: self.commit.to_owned(),
            },
            has_ref,
        ))
    }

    /// Returns a relative PathBuf where this Git repository
    /// should be stored within the storage directory.
    fn stored_path(&self, storage_dir: &Path) -> PathBuf {
        storage_dir.join("git").join(self.directory_name())
    }

    /// Returns the name of the directory that should contain
    /// the Git repository.
    /// It is a composition of the hostname and the repository name
    /// so that it's unique.
    fn directory_name(&self) -> PathBuf {
        let host = self.url.host_str();
        let path = self.url.path();

        let mut name = String::with_capacity(host.unwrap_or("").len() + 1 + path.len());
        if let Some(host) = host {
            name.push_str(host);
            name.push('_');
        }
        name.push_str(&path.trim_start_matches('/').replace('/', "."));
        name.into()
    }
}

/// Information available after [Git] is stored on disk.
pub struct StoredGit {
    /// Name of the upstream, as returned by [Git::name].
    pub name: String,
    /// Whether the stored Git repository was
    /// synchronized with [Git],
    /// that is, it existed and contained [Git::commit].
    pub was_cached: bool,
    repo: gitwrap::Repository,
    commit: String,
}

impl StoredGit {
    /// Shares the Git repository in preparation of a build.
    pub async fn share(&self, dest_dir: &Path) -> Result<SharedGit, Error> {
        if let Some(parent) = dest_dir.parent() {
            fs::create_dir_all(parent)?;
        }

        // Clone from our mirror to destdir
        let cloned = self.repo.clone_to(dest_dir).await?;

        // Cloning sets origin to the local mirror, but we want to use
        // the original remote as submodule resolving may depend on this
        let source_origin = self.repo.get_remote_url("origin").await?;
        cloned.set_remote_url("origin", &source_origin).await?;

        // Finally checkout the desired commit
        cloned.checkout(&self.commit).await?;

        Ok(SharedGit(dest_dir.to_owned()))
    }
}

/// A shared Git repository is a copy of a stored Git
/// in a location useful for a build.
pub struct SharedGit(PathBuf);

impl SharedGit {
    /// Removes the shared Git repository.
    pub fn remove(&self) -> Result<(), Error> {
        fs::remove_dir_all(&self.0).map_err(Error::from)
    }
}

/// Possible errors returned by functions in this module.
#[derive(Debug, Error)]
pub enum Error {
    /// An error occurred while handling a Git repository.
    #[error("{0}")]
    Git(#[from] gitwrap::Error),
    /// A generic I/O error occurred.
    #[error("{0}")]
    Io(#[from] io::Error),
}

async fn clone(url: &Url, path: &Path, pb: &ProgressBar) -> Result<gitwrap::Repository, gitwrap::Error> {
    let cb = set_progress_bar_style(pb);

    let result = gitwrap::Repository::clone_mirror_progress(path, url, cb).await;
    pb.finish_and_clear();

    result
}

async fn fetch(repo: &gitwrap::Repository, pb: &ProgressBar) -> Result<(), gitwrap::Error> {
    let cb = set_progress_bar_style(pb);

    let result = repo.fetch_progress(cb).await;
    pb.finish_and_clear();

    result
}

fn set_progress_bar_style(pb: &ProgressBar) -> impl Fn(gitwrap::FetchProgress) {
    pb.set_length(100);
    pb.set_style(
        ProgressStyle::with_template(" {spinner} |{percent:>3}%| {wide_msg} {prefix:>.dim} ")
            .unwrap()
            .tick_chars("--=≡■≡=--"),
    );

    |prog| {
        pb.set_position(prog.percent as u64);
        pb.set_prefix(prog.speed);
    }
}
