// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
    string,
};

use fs_err as fs;
use moss::util;
use thiserror::Error;
use tui::{ProgressBar, ProgressStyle};
use url::Url;

#[derive(Clone, Debug)]
pub struct Git {
    pub url: Url,
    pub ref_id: String,
}

impl Git {
    pub async fn fetch_new(url: &Url, container_dir: &Path) -> Result<Self, Error> {
        Self::fetch_new_progress(url, container_dir, &ProgressBar::hidden()).await
    }

    pub async fn fetch_new_progress(url: &Url, container_dir: &Path, pb: &ProgressBar) -> Result<Self, Error> {
        todo!()
    }

    pub fn name(&self) -> &str {
        util::uri_file_name(&self.url)
    }

    pub async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<StoredGit, Error> {
        use fs_err::tokio as fs;

        match self.stored(storage_dir) {
            Ok((stored, has_ref)) => {
                if !has_ref {
                    stored.repo.update(&[&self.ref_id])?;
                }
                Ok(stored)
            }
            Err(Error::Git(e)) if matches!(e.code(), git2::ErrorCode::NotFound) => {
                let dir = storage_dir.join(self.directory_name());

                fs::create_dir_all(storage_dir).await?;
                match clone(&self.url, &dir, pb) {
                    Ok(repo) => Ok(StoredGit {
                        name: self.name().to_owned(),
                        was_cached: false,
                        repo,
                        commit: self.ref_id.to_owned(),
                    }),
                    Err(e) => {
                        let _ = fs::remove_dir_all(&dir).await;
                        Err(Error::from(e))
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn stored(&self, storage_dir: &Path) -> Result<(StoredGit, bool), Error> {
        let dir = storage_dir.join(self.directory_name());

        let repo = git2wrap::Repository::open_bare(&dir)?;
        let has_ref = repo.has_commit(&self.ref_id);
        Ok((
            StoredGit {
                name: self.name().to_owned(),
                was_cached: has_ref,
                repo,
                commit: self.ref_id.to_owned(),
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

pub struct StoredGit {
    pub name: String,
    pub was_cached: bool,
    repo: git2wrap::Repository,
    commit: String,
}

impl StoredGit {
    pub fn share(&self, dest_dir: &Path) -> Result<SharedGit, Error> {
        if let Some(parent) = dest_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(SharedGit(
            self.repo
                .add_worktree(dest_dir, &self.repo.reference(&self.reference)?)?,
        ))
    }

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

pub struct SharedGit(git2wrap::Worktree);

impl SharedGit {
    pub fn remove(&self) -> Result<(), Error> {
        self.0.remove().map_err(Error::from)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Git(#[from] gitwrap::Error),
    #[error("{0}")]
    Io(#[from] io::Error),
}

async fn clone(url: &Url, path: &Path, pb: &ProgressBar) -> Result<gitwrap::Repository, gitwrap::Error> {
    use tui::HumanBytes;

    pb.set_length(100);
    pb.set_style(progress_bar_style());

    let result = gitwrap::Repository::clone_bare_progress(path, url, |prog| {
        pb.set_position(prog.percent as u64);
        pb.set_message(HumanBytes(prog.speed).to_string());
    })
    .await;
    pb.finish_and_clear();

    result
}

async fn fetch(repo: &gitwrap::Repository, pb: &ProgressBar) -> Result<(), gitwrap::Error> {
    use tui::HumanBytes;

    pb.set_length(100);
    pb.set_style(progress_bar_style());

    let result = repo
        .fetch_progress(|prog| {
            pb.set_position(prog.percent as u64);
            pb.set_message(HumanBytes(prog.speed).to_string());
        })
        .await;
    pb.finish_and_clear();

    result
}

fn progress_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(" {spinner} {msg}/s ")
        .unwrap()
        .tick_chars("--=≡■≡=--")
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("ref '{ref_id}' did not resolve to a valid commit hash for {uri}")]
    UnresolvedRef { ref_id: String, uri: Url },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Utf8(#[from] string::FromUtf8Error),
}
