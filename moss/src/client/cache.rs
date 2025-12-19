// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Cache management for unpacking remote assets (`.stone`, etc.)

use std::collections::HashSet;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use futures_util::StreamExt;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

use stone::{payload, read::PayloadKind};

use crate::{Installation, package, request};

/// Synchronized set of assets that are currently being
/// unpacked. Used to prevent unpacking the same asset
/// from different packages at the same time.
#[derive(Debug, Clone, Default)]
pub struct UnpackingInProgress(Arc<Mutex<HashSet<PathBuf>>>);

/// RAII guard representing exclusive ownership of an
/// in-progress asset unpack operation. When dropped
/// the asset is automatically removed from the
/// in-progress set.
pub struct InProgressGuard {
    owner: UnpackingInProgress,
    path: Option<PathBuf>,
}

impl UnpackingInProgress {
    /// Attempt to acquire exclusive unpack ownership for the asset.
    ///
    /// Returns `Some(InProgressGuard)` if the asset was successfully
    /// acquired, or `None` if another worker is currently unpacking it.
    pub fn acquire(&self, path: PathBuf) -> Option<InProgressGuard> {
        let mut lock = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if lock.insert(path.clone()) {
            Some(InProgressGuard {
                owner: self.clone(),
                path: Some(path),
            })
        } else {
            None
        }
    }
}

/// Removes the asset from the in-progress set when the guard
/// goes out of scope.
impl Drop for InProgressGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let mut lock = self.owner.0.lock().unwrap_or_else(|e| e.into_inner());
            lock.remove(&path);
        }
    }
}

/// Per-package progress tracking for UI integration
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub delta: u64,
    pub completed: u64,
    pub total: u64,
}

impl Progress {
    /// Return the completion as a percentage
    pub fn pct(&self) -> f32 {
        self.completed as f32 / self.total as f32
    }
}

/// Fetch a package with the provided [`package::Meta`] and [`Installation`] and return a [`Download`] on success.
pub async fn fetch(
    meta: &package::Meta,
    installation: &Installation,
    on_progress: impl Fn(Progress),
) -> Result<Download, Error> {
    use fs_err::tokio::{self as fs, File};

    let url = meta.uri.as_ref().ok_or(Error::MissingUri)?.parse::<Url>()?;
    let hash = meta.hash.as_ref().ok_or(Error::MissingHash)?;

    let destination_path = download_path(installation, hash)?;
    let partial_path = destination_path.with_extension("part");

    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    if tokio::fs::try_exists(&destination_path).await? {
        return Ok(Download {
            id: meta.id().into(),
            path: destination_path,
            installation: installation.clone(),
            was_cached: true,
        });
    }

    let mut bytes = request::get(url).await?;
    let mut out = File::create(&partial_path).await?;

    let mut total = 0;

    while let Some(chunk) = bytes.next().await {
        let bytes = chunk?;
        let delta = bytes.len() as u64;
        total += delta;
        out.write_all(&bytes).await?;

        (on_progress)(Progress {
            delta,
            completed: total,
            total: meta.download_size.unwrap_or(total),
        });
    }

    out.flush().await?;

    fs::rename(partial_path, &destination_path).await?;

    Ok(Download {
        id: meta.id().into(),
        path: destination_path,
        installation: installation.clone(),
        was_cached: false,
    })
}

/// A package that has been downloaded to the installation
pub struct Download {
    id: package::Id,
    path: PathBuf,
    installation: Installation,
    pub was_cached: bool,
}

/// Upon fetch completion we have this unpacked asset bound with
/// an open reader
pub struct UnpackedAsset {
    pub payloads: Vec<PayloadKind>,
}

impl Download {
    /// Unpack the downloaded package
    // TODO: Return an "Unpacked" struct which has a "blit" method on it?
    pub fn unpack(
        self,
        unpacking_in_progress: UnpackingInProgress,
        on_progress: impl Fn(Progress) + Send + 'static,
    ) -> Result<UnpackedAsset, Error> {
        use fs_err::{self as fs, File};
        use std::io::{self, Read, Seek, SeekFrom, Write};

        struct ProgressWriter<'a, W> {
            writer: W,
            total: u64,
            written: u64,
            on_progress: &'a dyn Fn(Progress),
        }

        impl<'a, W> ProgressWriter<'a, W> {
            pub fn new(writer: W, total: u64, on_progress: &'a impl Fn(Progress)) -> Self {
                Self {
                    writer,
                    total,
                    written: 0,
                    on_progress,
                }
            }
        }

        impl<W: Write> Write for ProgressWriter<'_, W> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let bytes = self.writer.write(buf)?;

                self.written += bytes as u64;

                (self.on_progress)(Progress {
                    delta: bytes as u64,
                    completed: self.written,
                    total: self.total,
                });

                Ok(bytes)
            }

            fn flush(&mut self) -> io::Result<()> {
                self.writer.flush()
            }
        }

        let content_dir = self.installation.cache_path("content");
        let content_path = content_dir.join(self.id);

        fs::create_dir_all(&content_dir)?;

        let mut reader = stone::read(File::open(&self.path)?)?;

        let payloads = reader.payloads()?.collect::<Result<Vec<_>, _>>()?;
        let indices = payloads
            .iter()
            .filter_map(PayloadKind::index)
            .flat_map(|p| &p.body)
            .collect::<Vec<_>>();

        // If we don't have any files to unpack OR download was cached
        // & all assets exist, we can skip unpacking
        if indices.is_empty() || (self.was_cached && check_assets_exist(&indices, &self.installation)) {
            return Ok(UnpackedAsset { payloads });
        }

        let content = payloads
            .iter()
            .find_map(PayloadKind::content)
            .ok_or(Error::MissingContent)?;

        let content_file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&content_path)?;

        reader.unpack_content(
            content,
            &mut ProgressWriter::new(&content_file, content.header.plain_size, &on_progress),
        )?;

        indices
            .into_iter()
            .map(|idx| {
                let path = asset_path(&self.installation, &format!("{:02x}", idx.digest));

                // Acquire in-progress guard.
                let _guard = match unpacking_in_progress.acquire(path.clone()) {
                    Some(guard) => guard,
                    None => return Ok(()),
                };

                // This asset already exists
                if path.exists() {
                    return Ok(());
                }

                // Create parent dir
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Split file reader over index range
                let mut file = &content_file;
                file.seek(SeekFrom::Start(idx.start))?;
                let mut split_file = (&mut file).take(idx.end - idx.start);

                let mut output = File::create(&path)?;

                io::copy(&mut split_file, &mut output)?;

                Ok(())
            })
            .collect::<Result<Vec<_>, Error>>()?;

        fs::remove_file(&content_path)?;

        Ok(UnpackedAsset { payloads })
    }
}

/// Returns true if all assets already exist in the installation
fn check_assets_exist(indices: &[&payload::Index], installation: &Installation) -> bool {
    indices.iter().all(|index| {
        let path = asset_path(installation, &format!("{:02x}", index.digest));
        path.exists()
    })
}

/// Returns a fully qualified filesystem path to download the given hash ID into
pub fn download_path(installation: &Installation, hash: &str) -> Result<PathBuf, Error> {
    if hash.len() < 5 {
        return Err(Error::MalformedHash(hash.to_owned()));
    }

    let directory = installation
        .cache_path("downloads")
        .join("v1")
        .join(&hash[..5])
        .join(&hash[hash.len() - 5..]);

    Ok(directory.join(hash))
}

/// Returns a fully qualified filesystem path to promote the final asset into
pub fn asset_path(installation: &Installation, hash: &str) -> PathBuf {
    let directory = if hash.len() >= 10 {
        installation
            .assets_path("v2")
            .join(&hash[..2])
            .join(&hash[2..4])
            .join(&hash[4..6])
    } else {
        installation.assets_path("v2")
    };

    directory.join(hash)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Missing download hash")]
    MissingHash,
    #[error("Missing download URI")]
    MissingUri,
    #[error("Missing content payload")]
    MissingContent,
    #[error("Malformed download hash: {0}")]
    MalformedHash(String),
    #[error("stone format")]
    Format(#[from] stone::read::Error),
    #[error("invalid url")]
    InvalidUrl(#[from] url::ParseError),
    #[error("request")]
    Request(#[from] request::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
