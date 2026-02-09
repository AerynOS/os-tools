// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    num::NonZeroUsize,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    thread,
};

use fs_err as fs;
use nix::unistd::{LinkatFlags, linkat};
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
