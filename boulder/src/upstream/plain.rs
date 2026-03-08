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

#[derive(Debug, Clone)]
pub struct Plain {
    pub url: Url,
    pub hash: Hash,
    pub rename: Option<String>,
}

impl Plain {
    pub async fn fetch_new(url: Url, dest_file: &Path) -> Result<Self, Error> {
        Self::fetch_new_progress(url, dest_file, &ProgressBar::hidden()).await
    }

    pub async fn fetch_new_progress(url: Url, dest_file: &Path, pb: &ProgressBar) -> Result<Self, Error> {
        let hash = fetch(url.clone(), dest_file, pb).await?;
        Ok(Self {
            url,
            hash,
            rename: None,
        })
    }

    pub fn name(&self) -> &str {
        if let Some(name) = &self.rename {
            name
        } else {
            util::uri_file_name(&self.url)
        }
    }

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

    /// Returns an already-stored archive.
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

    /// Returns a relative PathBuf where this archive should be stored
    /// within the recipe storage.
    pub fn stored_path(&self, storage_dir: &Path) -> PathBuf {
        [storage_dir, &self.file_path()].iter().collect()
    }

    /// Returns a relative PathBuf based on the hash of the archive's URL
    /// and the archive's very hash.
    ///
    /// Hashing this data ensures the path is unique and becomes invalid
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

#[derive(Clone)]
pub struct StoredPlain {
    pub name: String,
    pub path: PathBuf,
    pub was_cached: bool,
}

impl StoredPlain {
    pub fn share(&self, dest_dir: &Path) -> Result<SharedPlain, Error> {
        let target = dest_dir.join(self.name.clone());

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

    pub fn remove(&self) -> Result<(), Error> {
        fs::remove_file(&self.path)?;

        let parents = self.path.parent().unwrap_or(Path::new("")).iter();
        for parent in parents.rev() {
            match fs::remove_dir(parent) {
                Ok(_) => {},
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => break,
                Err(e) => return Err(Error::from(e))
            }
        }

        Ok(())
    }
}

pub struct SharedPlain {
    pub path: PathBuf,
}

impl SharedPlain {
    pub fn remove(&self) -> Result<(), Error> {
        fs::remove_file(&self.path).map_err(Error::from)
    }
}

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

#[derive(Debug, Error)]
pub enum ParseHashError {
    #[error("hash too short: {0}")]
    TooShort(String),
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("parse hash")]
    ParseHash(#[from] ParseHashError),
    #[error("hash mismatch for {name}, expected {expected:?} got {:?}", got.0)]
    HashMismatch { name: String, expected: String, got: Hash },
    #[error("request")]
    Request(#[from] request::Error),
    #[error("io")]
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
