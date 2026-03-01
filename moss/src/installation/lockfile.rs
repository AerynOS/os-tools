// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fmt,
    fs::File,
    io::{self},
    path::PathBuf,
    sync::Arc,
};

use nix::fcntl::{Flock, FlockArg};
use thiserror::Error;

/// An acquired file lock guaranteeing exclusive access
/// to the underlying directory.
///
/// The lock is automatically released once all instances
/// of this ref counted lock are dropped.
#[derive(Debug, Clone)]
#[allow(unused)]
pub struct Lock(Arc<Flock<File>>);

/// Acquires a file lock at the provided path. If the file is currently
/// locked, `block_msg` will be displayed and the function will block
/// until the lock is released.
///
/// Returns the acquired [`Lock`] that will be held until dropped.
pub fn acquire(path: impl Into<PathBuf>, block_msg: impl fmt::Display) -> Result<Lock, Error> {
    let path = path.into();

    let file = File::options().create(true).write(true).truncate(false).open(path)?;

    let file = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => lock,
        Err((file, nix::errno::Errno::EWOULDBLOCK)) => {
            println!("{block_msg}");
            Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, e)| e)?
        }
        Err((_, e)) => return Err(e.into()),
    };

    Ok(Lock(Arc::new(file)))
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),
    #[error("obtaining exclusive file lock")]
    Flock(#[from] nix::errno::Errno),
}
