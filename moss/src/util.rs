// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io::{self, Read, Seek, Write},
    num::NonZeroUsize,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    thread,
};

use fs_err as fs;
use nix::unistd::{LinkatFlags, linkat};
use rayon::iter::{ParallelBridge, ParallelIterator};
use sha2::{Digest, Sha256};
use stone::{StoneDecodedPayload, StoneReadError};
use url::Url;

pub fn ensure_dir_exists(path: &Path) -> io::Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

pub fn recreate_dir(path: &Path) -> io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

pub fn copy_dir(source_dir: &Path, out_dir: &Path) -> io::Result<()> {
    recreate_dir(out_dir)?;

    let contents = fs::read_dir(source_dir)?;

    for entry in contents.flatten() {
        let path = entry.path();

        if let Some(file_name) = path.file_name() {
            let dest = out_dir.join(file_name);
            let meta = entry.metadata()?;

            if meta.is_dir() {
                copy_dir(&path, &dest)?;
            } else if meta.is_file() {
                fs::copy(&path, &dest)?;
            } else if meta.is_symlink() {
                symlink(fs::read_link(&path)?, &dest)?;
            }
        }
    }

    Ok(())
}

pub fn enumerate_files<'a>(
    dir: &'a Path,
    matcher: impl Fn(&Path) -> bool + Send + Copy + 'a,
) -> io::Result<Vec<PathBuf>> {
    let read_dir = fs::read_dir(dir)?;

    let mut paths = vec![];

    for entry in read_dir {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            paths.extend(enumerate_files(&path, matcher)?);
        } else if meta.is_file() && matcher(&path) {
            paths.push(path);
        }
    }

    Ok(paths)
}

pub fn list_dirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let read_dir = fs::read_dir(dir)?;

    let mut paths = vec![];

    for entry in read_dir.flatten() {
        let path = entry.path();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            paths.push(path);
        }
    }

    Ok(paths)
}

pub fn hardlink_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    // Attempt hard link
    let link_result = linkat(None, from, None, to, LinkatFlags::NoSymlinkFollow);

    // Copy instead
    if link_result.is_err() {
        fs::copy(from, to)?;
    }

    Ok(())
}

pub async fn async_hardlink_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    let from = from.to_owned();
    let to = to.to_owned();

    tokio::task::spawn_blocking(move || hardlink_or_copy(&from, &to))
        .await
        .expect("join handle")
}

pub fn uri_file_name(uri: &Url) -> &str {
    let path = uri.path();

    path.rsplit('/').next().unwrap_or_default()
}

pub fn uri_relative_path(uri: &Url) -> &str {
    let path = uri.path();

    path.strip_prefix('/').unwrap_or_default()
}

pub fn num_cpus() -> NonZeroUsize {
    thread::available_parallelism().unwrap_or_else(|_| NonZeroUsize::new(1).unwrap())
}

pub fn is_root() -> bool {
    use nix::unistd::Uid;

    Uid::effective().is_root()
}

/// Remove all empty folders from `starting` and moving up until `root`
///
/// `root` must be a prefix / ancestor of `starting`
pub fn remove_empty_dirs(starting: &Path, root: &Path) -> io::Result<()> {
    if !starting.starts_with(root) || !starting.is_dir() || !root.is_dir() {
        return Ok(());
    }

    let mut current = Some(starting);

    while let Some(dir) = current.take() {
        if dir.exists() {
            let is_empty = fs::read_dir(dir)?.count() == 0;

            if !is_empty {
                return Ok(());
            }

            fs::remove_dir(dir)?;
        }

        if let Some(parent) = dir.parent()
            && parent != root
        {
            current = Some(parent);
        }
    }

    Ok(())
}

/// Removes a directory at this path, after removing all its contents. Use carefully!
/// If root `path` is not found return Ok, this avoids having to check the root directory
/// exists first, avoiding a TOCTOU.
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    Ok(fs::remove_dir_all(path).or_else(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(e)
        }
    })?)
}

/// Removes a directory at this path, after removing all its contents in parallel. Use carefully!
///
/// Attempts to match std::fs::remove_dir_all as close as possible whilst also ignoring `NotFound`
/// error if the root `path` does not exist.
pub fn par_remove_dir_all(path: &Path) -> io::Result<()> {
    let rayon_runtime = rayon::ThreadPoolBuilder::new().build().expect("rayon runtime");

    rayon_runtime.install(|| -> io::Result<()> {
        let filetype = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata.file_type(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        if filetype.is_symlink() {
            fs::remove_file(path)
        } else {
            par_remove_dir_all_recursive(path)
        }
    })?;
    Ok(())
}

fn par_remove_dir_all_recursive(path: &Path) -> io::Result<()> {
    fs::read_dir(path)?
        .par_bridge()
        // TODO: Use try {} here once it becomes stable to match stdlib
        //       and simplify error handling
        .try_for_each(|child| -> io::Result<()> {
            let child = match child {
                Ok(c) => c,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };

            let child_path = child.path();

            let result = if child.file_type()?.is_dir() {
                par_remove_dir_all_recursive(&child_path)
            } else {
                fs::remove_file(&child_path)
            };

            match result {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        })?;

    ignore_notfound(fs::remove_dir(path))
}

fn ignore_notfound<T>(result: io::Result<T>) -> io::Result<()> {
    match result {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    }
}

/// Computes the sha256 hash of the provided reader
pub fn sha256_hash<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut writer = Sha256Wrapper::new(io::sink());

    io::copy(reader, &mut writer)?;

    Ok(writer.finalize())
}

/// Wraps an inner reader or writer and provides
/// a `finalize` method to produce a sha256 hash
/// from the read / written bytes
pub struct Sha256Wrapper<T> {
    inner: T,
    hasher: Sha256,
}

impl<T> Sha256Wrapper<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            hasher: Sha256::default(),
        }
    }

    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

impl<T: Read> Read for Sha256Wrapper<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.hasher.update(&buf);
        self.inner.read(buf)
    }
}

impl<T: Write> Write for Sha256Wrapper<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.hasher.update(buf);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Extract stone payloads from the provided reader
pub fn stone_payloads<R: Read + Seek>(reader: &mut R) -> Result<Vec<StoneDecodedPayload>, StoneReadError> {
    stone::read(reader)?.payloads()?.collect::<Result<Vec<_>, _>>()
}
