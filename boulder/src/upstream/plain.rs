// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

use futures_util::StreamExt;
use moss::{request, runtime, util};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt};
use tui::{ProgressBar, ProgressStyle};
use url::Url;

use crate::Paths;

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
        let hash = Self::fetch(&url, dest_file, pb).await?;
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

    fn path(&self, paths: &Paths) -> PathBuf {
        // Hash uri and file hash together
        // for a unique file path that can
        // be used for caching purposes and
        // is busted if either uri or hash
        // change
        let mut hasher = Sha256::new();
        hasher.update(self.url.as_str());
        hasher.update(&self.hash.0);

        let hash = hex::encode(hasher.finalize());

        paths
            .upstreams()
            .host
            .join("fetched")
            // Type safe guaranteed to be >= 5 bytes
            .join(&hash[..5])
            .join(&hash[hash.len() - 5..])
            .join(hash)
    }

    async fn fetch(url: &Url, dest_file: &Path, pb: &ProgressBar) -> Result<Hash, Error> {
        pb.set_style(
            ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
                .unwrap()
                .tick_chars("--=≡■≡=--"),
        );

        let mut stream = request::stream(url.clone()).await?;
        let mut hasher = Sha256::new();
        let mut out = File::create(&dest_file).await?;

        while let Some(chunk) = stream.next().await {
            let bytes = &chunk?;
            pb.inc(bytes.len() as u64);
            hasher.update(bytes);
            out.write_all(bytes).await?;
        }
        out.flush().await?;

        Ok(hex::encode(hasher.finalize()).try_into()?)
    }

    pub async fn store(&self, paths: &Paths, pb: &ProgressBar) -> Result<StoredPlain, Error> {
        use tokio::fs;

        pb.set_style(
            ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
                .unwrap()
                .tick_chars("--=≡■≡=--"),
        );

        let name = self.name();
        let path = self.path(paths);
        let partial_path = path.with_extension("part");

        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }

        if path.exists() {
            return Ok(StoredPlain {
                name: name.to_owned(),
                path,
                was_cached: true,
            });
        }

        let hash = Self::fetch(&self.url, &path, pb).await?;
        if hash != self.hash {
            fs::remove_file(&partial_path).await?;

            return Err(Error::HashMismatch {
                name: name.to_owned(),
                expected: self.hash.0.clone(),
                got: hash,
            });
        }

        fs::rename(partial_path, &path).await?;

        Ok(StoredPlain {
            name: name.to_owned(),
            path,
            was_cached: false,
        })
    }

    pub fn remove(&self, paths: &Paths) -> Result<(), Error> {
        let path = self.path(paths);

        fs::remove_file(&path)?;

        if let Some(parent) = path.parent() {
            util::remove_empty_dirs(parent, &paths.upstreams().host)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct StoredPlain {
    pub name: String,
    pub path: PathBuf,
    pub was_cached: bool,
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
        Self::from_str(value.as_str())
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
    #[error("hash mismatch for {name}, expected {expected:?} got {got:?}")]
    HashMismatch { name: String, expected: String, got: Hash },
    #[error("request")]
    Request(#[from] request::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
