// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::ops::Deref;
use std::{
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

use fs_err as fs;
use moss::{request, util};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tui::{ProgressBar, ProgressStyle};
use url::Url;

/// Upstream based on an archive (typically a tarball).
#[derive(Debug, Clone)]
pub struct Plain {
    /// URL from where the source archive is fetched.
    pub url: Url,
    /// SHA256 hash of the archive.
    pub hash: Hash,
    /// Name of the upstream when stored in the storage
    /// directory. If None, a default name is implied from [Self::url].
    pub rename: Option<String>,
}

impl Plain {
    /// Fetches a new source archive into a destination file path.
    /// A local URL (i.e. starting with `file://`) is allowed; in that case
    /// the source archived is copied.
    pub async fn fetch_new(url: Url, dest_file: &Path) -> Result<Self, Error> {
        Self::fetch_new_progress(url, dest_file, &ProgressBar::hidden()).await
    }

    /// Fetches a new source archive into a destination file path.
    /// It is identical to [Self::fetch_new], but accepts a progress bar
    /// for progress reporting.
    pub async fn fetch_new_progress(url: Url, dest_file: &Path, pb: &ProgressBar) -> Result<Self, Error> {
        let hash = fetch(url.clone(), dest_file, pb).await?;
        Ok(Self {
            url,
            hash,
            rename: None,
        })
    }

    /// Returns the name of the source archive.
    /// If [Self::rename] is not defined, it is implied from the URL.
    pub fn name(&self) -> &str {
        if let Some(name) = &self.rename {
            name
        } else {
            util::uri_file_name(&self.url)
        }
    }

    /// Stores the source archive into the storage directory.
    ///
    /// If the upstream was already stored and [Self::hash] matches,
    /// no write operation takes place. If the source archive was
    /// not stored or the hash does not match, it is overwritten.
    pub async fn store(&self, storage_dir: &Path, pb: &ProgressBar) -> Result<StoredPlain, Error> {
        use fs_err::tokio as fs;

        match self.stored(storage_dir) {
            Ok(stored) => return Ok(stored),
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
            Err(Error::HashMismatch { .. }) => {}
            Err(err) => return Err(err),
        }

        let path = self.stored_path(storage_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(&parent).await?;
        }

        let hash = fetch(self.url.clone(), &path, pb).await?;
        if hash != self.hash {
            fs::remove_file(&path).await?;

            return Err(Error::HashMismatch {
                name: self.name().to_owned(),
                expected: self.hash.to_string(),
                got: hash,
            });
        }

        Ok(StoredPlain {
            name: self.name().to_owned(),
            path,
            was_cached: false,
        })
    }

    /// Returns an already-stored source archive.
    /// An error is instead returned if the source archive is
    /// not found in the storage directory, or its hash doesn't match
    /// [Self::hash].
    pub fn stored(&self, storage_dir: &Path) -> Result<StoredPlain, Error> {
        let path = self.stored_path(storage_dir);

        let mut file = fs_err::File::open(&path)?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        let hash = hex::encode(hasher.finalize());
        if hash != self.hash.deref() {
            return Err(Error::HashMismatch {
                name: self.name().to_owned(),
                expected: self.hash.to_string(),
                got: Hash(hash),
            });
        }

        Ok(StoredPlain {
            name: self.name().to_owned(),
            path,
            was_cached: true,
        })
    }

    /// Returns a relative PathBuf where this source archive
    /// should be stored within the storage directory.
    pub fn stored_path(&self, storage_dir: &Path) -> PathBuf {
        [storage_dir, &self.file_path()].iter().collect()
    }

    /// Returns a relative PathBuf based on the hashes of [Self::url]
    /// and [Self::hash].
    ///
    /// Hashing both ensures the path is unique and becomes invalid
    /// as soon as either the URL or the hash changes.
    fn file_path(&self) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(self.url.as_str());
        hasher.update(self.hash.as_bytes());

        let hash = hex::encode(hasher.finalize());
        // Type safe guaranteed to be >= 5 bytes.
        [&hash[..5], &hash[hash.len() - 5..], &hash].iter().collect()
    }
}

/// Information available after [Plain] is stored on disk.
#[derive(Clone)]
pub struct StoredPlain {
    /// Name of the upstream, as returned by [Plain::name].
    pub name: String,
    /// Path of the source archive after it was stored.
    pub path: PathBuf,
    /// Whether the source archived was already stored with valid hash.
    pub was_cached: bool,
}

impl StoredPlain {
    /// Shares the Git repository in preparation of a build.
    ///
    /// This function tries to be as efficient as possible in terms
    /// of actual bytes copied: a hard link is created if possible.
    pub fn share(&self, dest_dir: &Path) -> Result<SharedPlain, Error> {
        let target = dest_dir.join(self.name.clone());

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        // Attempt hard link.
        let result = fs::hard_link(&self.path, &target);
        if let Err(err) = &result
            && err.kind() == io::ErrorKind::CrossesDevices
        {
            // Source and destination paths
            // reside on different filesystems.
            // Copy it instead.
            fs::copy(&self.path, &target).map(|_| ())
        } else {
            result
        }?;

        Ok(SharedPlain { path: target })
    }

    /// Removes the stored source archive.
    /// Should the archive no longer exist,
    /// this function returns successfully (it is idempotent).
    pub fn remove(&self) -> Result<(), Error> {
        fs::remove_file(&self.path)?;

        let parents = self.path.parent().unwrap_or(Path::new("")).iter();
        for parent in parents.rev() {
            match fs::remove_dir(parent) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => break,
                Err(e) => return Err(Error::from(e)),
            }
        }

        Ok(())
    }
}

/// A shared source archive is a copy of a stored source archive
/// in a location useful for a build.
pub struct SharedPlain {
    /// Path of the source archive after it was shared.
    pub path: PathBuf,
}

impl SharedPlain {
    /// Removes the shared source archive.
    /// Should the archive no longer exist,
    /// this function returns successfully (it is idempotent).
    pub fn remove(&self) -> Result<(), Error> {
        fs::remove_file(&self.path).map_err(Error::from)
    }
}

/// Thin wrapper around String that represents a
/// hexadecimal SHA256 hash.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Hash(String);

impl FromStr for Hash {
    type Err = ParseHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < 5 {
            return Err(ParseHashError::TooShort(s.to_owned()));
        }

        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<String> for Hash {
    type Error = ParseHashError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() < 5 {
            return Err(ParseHashError::TooShort(value));
        }
        Ok(Self(value))
    }
}

impl Deref for Hash {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

/// Reasons why [Hash] may be invalid.
#[derive(Debug, Error)]
pub enum ParseHashError {
    #[error("hash too short: {0}")]
    TooShort(String),
}

/// Possible errors returned by functions in this module.
#[derive(Debug, Error)]
pub enum Error {
    /// [Hash] is malformed.
    #[error("parse hash")]
    ParseHash(#[from] ParseHashError),
    /// Two hashes did not match.
    #[error("hash mismatch for {name}, expected {expected:?} got {:?}", got.0)]
    HashMismatch { name: String, expected: String, got: Hash },
    #[error("request")]
    /// A local or remote fetch failed.
    Request(#[from] request::Error),
    #[error("io")]
    /// A generic I/O error occured.
    Io(#[from] io::Error),
}

async fn fetch(url: Url, dest: &Path, pb: &ProgressBar) -> Result<Hash, Error> {
    pb.set_style(
        ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
            .unwrap()
            .tick_chars("--=≡■≡=--"),
    );

    request::download_with_progress_and_sha256(url, dest, |progress| pb.inc(progress.delta))
        .await
        .map_err(Error::from)?
        .try_into()
        .map_err(Error::from)
}
