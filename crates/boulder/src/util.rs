// SPDX-FileCopyrightText: Copyright © 2020-2023 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    thread,
};

use futures::{future::BoxFuture, FutureExt};
use tokio::fs::{copy, create_dir_all, read_dir, read_link, remove_dir_all, symlink};
use url::Url;

pub async fn ensure_dir_exists(path: &Path) -> Result<(), io::Error> {
    if !path.exists() {
        create_dir_all(path).await?;
    }
    Ok(())
}

pub async fn recreate_dir(path: &Path) -> Result<(), io::Error> {
    if path.exists() {
        remove_dir_all(path).await?;
    }
    create_dir_all(path).await?;
    Ok(())
}

pub fn copy_dir<'a>(
    source_dir: &'a Path,
    out_dir: &'a Path,
) -> BoxFuture<'a, Result<(), io::Error>> {
    async move {
        recreate_dir(out_dir).await?;

        let mut contents = read_dir(&source_dir).await?;

        while let Some(entry) = contents.next_entry().await? {
            let path = entry.path();

            if let Some(file_name) = path.file_name() {
                let dest = out_dir.join(file_name);
                let meta = entry.metadata().await?;

                if meta.is_dir() {
                    copy_dir(&path, &dest).await?;
                } else if meta.is_file() {
                    copy(&path, &dest).await?;
                } else if meta.is_symlink() {
                    symlink(read_link(&path).await?, &dest).await?;
                }
            }
        }

        Ok(())
    }
    .boxed()
}

pub async fn list_dirs(dir: &Path) -> Result<Vec<PathBuf>, io::Error> {
    let mut read_dir = read_dir(dir).await?;

    let mut paths = vec![];

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        let meta = entry.metadata().await?;

        if meta.is_dir() {
            paths.push(path);
        }
    }

    Ok(paths)
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

pub mod sync {
    use std::{
        fs::{create_dir_all, remove_dir_all},
        io,
        path::{Path, PathBuf},
    };

    pub fn ensure_dir_exists(path: &Path) -> Result<(), io::Error> {
        if !path.exists() {
            create_dir_all(path)?;
        }
        Ok(())
    }

    pub fn recreate_dir(path: &Path) -> Result<(), io::Error> {
        if path.exists() {
            remove_dir_all(path)?;
        }
        create_dir_all(path)?;
        Ok(())
    }

    pub fn enumerate_files<'a>(
        dir: &'a Path,
        matcher: impl Fn(&Path) -> bool + Send + Copy + 'a,
    ) -> Result<Vec<PathBuf>, io::Error> {
        use std::fs::read_dir;

        let read_dir = read_dir(dir)?;

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
}
