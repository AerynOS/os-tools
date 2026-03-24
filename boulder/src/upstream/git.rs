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
    /// Fetches a new Git upstream into a destination directory.
    /// A local URL (i.e. starting with `file://`) is allowed; in that case
    /// the Git repository is copied.
    pub async fn fetch_new(url: &Url, container_dir: &Path) -> Result<Self, Error> {
        Self::fetch_new_progress(url, container_dir, &ProgressBar::hidden()).await
    }

    /// Fetches a new Git upstream into a destination directory.
    /// It is identical to [Self::fetch_new], but accepts a progress bar
    /// for progress reporting.
    pub async fn fetch_new_progress(_url: &Url, _dest_dir: &Path, _pb: &ProgressBar) -> Result<Self, Error> {
        todo!()
    }

    /// Returns the name of the upstream. It is implied from the URL.
    pub fn name(&self) -> &str {
        util::uri_file_name(&self.url)
    }

    /// Stores the upstream into the storage directory.
    /// If the upstream was already stored but does not include [Self::commit],
    /// it is updated contextually. If it does not exist, the Git repository is cloned.
    pub async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<StoredGit, Error> {
        let dir = storage_dir.join(self.directory_name());

        let mut cached = true;
        let repo = match gitwrap::Repository::open_bare(&dir).await {
            Ok(repo) => {
                if !repo.has_commit(&self.commit).await? {
                    fetch(&repo, pb).await?;
                    cached = false;
                }
                repo
            }
            Err(e) => {
                if e.run_failed() {
                    cached = false;
                    clone(&self.url, &dir, pb).await?
                } else {
                    return Err(Error::from(e));
                }
            }
        };

        Ok(StoredGit {
            name: self.name().to_owned(),
            was_cached: cached,
            repo,
            commit: self.commit.to_owned(),
        })
    }

    /// Returns the stored upstream if it exists.
    ///
    /// If successful, a tuple is returned containing the
    /// stored upstream and a boolean flag, indicating whether
    /// the stored Git repository contains [Self::commit].
    pub async fn stored(&self, storage_dir: &Path) -> Result<(StoredGit, bool), Error> {
        let dir = storage_dir.join(self.directory_name());

        let repo = gitwrap::Repository::open_bare(&dir).await?;
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
    /// should be stored within the recipe storage.
    pub fn stored_path(&self, storage_dir: &Path) -> PathBuf {
        [storage_dir, &self.directory_name()].iter().collect()
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
            name.push('.');
        }
        name.push_str(&path.replace('/', "."));
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
    ///
    /// This function tries to be as efficient as possible in terms
    /// of actual bytes written/copied from the original Git repository.
    pub async fn share(&self, dest_dir: &Path) -> Result<SharedGit, Error> {
        if let Some(parent) = dest_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(SharedGit(self.repo.add_worktree(dest_dir, &self.commit).await?))
    }

    /// Removes the stored Git repository.
    /// Should the stored repository no longer exist,
    /// this function returns successfully (it is idempotent).
    pub fn remove(&self) -> Result<(), Error> {
        let result = fs::remove_dir_all(self.repo.path());
        if let Err(err) = result
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(Error::from(err));
        }
        Ok(())
    }
}

/// A shared Git repository is a copy of a stored Git
/// in a location useful for a build.
pub struct SharedGit(gitwrap::Worktree);

impl SharedGit {
    /// Removes the shared Git repository.
    /// Should the shared repository no longer exist,
    /// this function returns successfully (it is idempotent).
    pub async fn remove(&self) -> Result<(), Error> {
        self.0.remove().await.map_err(Error::from)
    }
}

/// Possible errors returned by functions in this module.
#[derive(Debug, Error)]
pub enum Error {
    /// An error occured while handling a Git repository.
    #[error("{0}")]
    Git(#[from] gitwrap::Error),
    /// A generic I/O error occured.
    #[error("{0}")]
    Io(#[from] io::Error),
}

async fn clone(url: &Url, path: &Path, pb: &ProgressBar) -> Result<gitwrap::Repository, gitwrap::Error> {
    let cb = set_progress_bar_style(pb);

    let result = gitwrap::Repository::clone_bare_progress(path, url, cb).await;
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
    use tui::HumanBytes;

    pb.set_length(100);
    pb.set_style(
        ProgressStyle::with_template("{prefix}\n|{bar:20.cyan/bue}| {percent}%, {msg}/s")
            .unwrap()
            .progress_chars("■≡=- "),
    );

    |prog| {
        pb.set_position(prog.percent as u64);
        pb.set_message(format!("{}", HumanBytes(prog.speed)));
    }
}
