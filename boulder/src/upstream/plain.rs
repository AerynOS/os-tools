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

    /// Unconditionally removes the source archive, and the parent
    /// directories if they are empty, within the storage directory.
    /// If the source archive does not exist, this function returns
    /// successfully (it is idempotent).
    ///
    /// Careful: this function does not validate the archive!
    /// It will be removed even if it does not belong to this Upstream.
    pub fn remove(&self, storage_dir: &Path) -> Result<(), Error> {
        let dir = self.stored_path(storage_dir);

        fs::remove_file(&dir)?;
        if let Some(parent) = dir.parent() {
            Ok(util::remove_empty_dirs(parent, storage_dir)?)
        } else {
            Ok(())
        }
    }

    /// Returns an already-stored source archive.
    /// An error is instead returned if the source archive is
    /// not found in the storage directory, or its hash doesn't match
    /// [Self::hash].
    pub fn stored(&self, storage_dir: &Path) -> Result<StoredPlain, Error> {
        let path = self.stored_path(storage_dir);

        let mut file = fs_err::File::open(&path)?;
        let hash = util::sha256_hash(&mut file)?;
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
    fn stored_path(&self, storage_dir: &Path) -> PathBuf {
        storage_dir.join("fetched").join(self.file_path())
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
        util::hardlink_or_copy(&self.path, &target)?;

        Ok(SharedPlain { path: target })
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
    /// A generic I/O error occurred.
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
