// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Encapsulation of a target installation filesystem

use std::path::{Path, PathBuf};

use fs_err as fs;
use log::{trace, warn};
use nix::unistd::{AccessFlags, Uid, access};
use thiserror::Error;
use tui::Styled;

use crate::state;

mod lockfile;

/// System mutability - do we have readwrite?
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum Mutability {
    /// We only have readonly access
    ReadOnly,
    /// We have read-write access
    ReadWrite,
}

/// Encapsulate details for a target installation filesystem
#[derive(Debug, Clone)]
pub struct Installation {
    /// Fully qualified root filesystem path
    pub root: PathBuf,

    /// Filesystem mutability: Will it be mutable or just RO for queries?
    pub mutability: Mutability,

    /// If present, the currently active system state Id
    pub active_state: Option<state::Id>,

    /// Custom cache directory location,
    /// otherwise derived from root
    pub cache_dir: Option<PathBuf>,

    /// Acquired locks that guarantee exclusive access
    /// to the installation for mutable operations
    _locks: Vec<lockfile::Lock>,
}

impl Installation {
    /// Open a system root as an Installation type
    /// This will query the potential active state if found,
    /// and determine the mutability per the current user identity
    /// and ACL permissions.
    pub fn open(root: impl Into<PathBuf>, cache_dir: Option<PathBuf>) -> Result<Self, Error> {
        let root: PathBuf = root.into();

        if !root.exists() || !root.is_dir() {
            return Err(Error::RootInvalid);
        }

        if let Some(dir) = &cache_dir
            && (!dir.exists() || !dir.is_dir())
        {
            return Err(Error::CacheInvalid);
        }

        // Make sure directories exist (silently fail if read-only)
        //
        // It's important we try this first in-case `root` needs to be created
        // as well, otherwise mutability will always be read-only
        // TODO: Should we instead fail if root doesn't exist?
        ensure_dirs_exist(&root);

        // Root? Always RW. Otherwise, check access for W
        let mutability = if Uid::effective().is_root() || access(&root, AccessFlags::W_OK).is_ok() {
            Mutability::ReadWrite
        } else {
            Mutability::ReadOnly
        };

        trace!("Mutability: {mutability}");
        trace!("Root dir: {root:?}");

        // Get exclusive access to work within these directories
        let _locks = if matches!(mutability, Mutability::ReadWrite) {
            acquire_locks(&root.join(".moss"), cache_dir.as_deref())?
        } else {
            vec![]
        };

        let active_state = read_state_id(&root);

        if let Some(id) = &active_state {
            trace!("Active State ID: {id}");
        } else {
            warn!("Unable to discover Active State ID");
        }

        Ok(Self {
            root,
            mutability,
            active_state,
            cache_dir: None,
            _locks,
        })
    }

    /// Return true if we lack write access
    pub fn read_only(&self) -> bool {
        matches!(self.mutability, Mutability::ReadOnly)
    }

    // Helper to form paths
    fn moss_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root.join(".moss").join(path)
    }

    /// Build a database path relative to the moss root
    pub fn db_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.moss_path("db").join(path)
    }

    /// Build a cache path relative to the moss root, or
    /// from the custom cache dir, if provided
    pub fn cache_path(&self, path: impl AsRef<Path>) -> PathBuf {
        if let Some(dir) = &self.cache_dir {
            dir.join(path)
        } else {
            self.moss_path("cache").join(path)
        }
    }

    /// Build an asset path relative to the moss root
    pub fn assets_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.moss_path("assets").join(path)
    }

    /// Build a repo path relative to the root
    pub fn repo_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.moss_path("repo").join(path)
    }

    /// Build a path relative to the moss system roots tree
    pub fn root_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.moss_path("root").join(path)
    }

    /// Build a staging path for in-progress system root transactions
    pub fn staging_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root_path("staging").join(path)
    }

    /// Return the staging directory itself
    pub fn staging_dir(&self) -> PathBuf {
        self.root_path("staging")
    }

    /// Return the container dir itself
    pub fn isolation_dir(&self) -> PathBuf {
        self.root_path("isolation")
    }

    /// Build a container path for isolated triggers
    pub fn isolation_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root_path("isolation").join(path)
    }
}

/// Blocks until lockfiles can be obtained for the
/// root `moss` path and if provided, the custom
/// cache path
///
/// Locks are held until dropped
pub fn acquire_locks(moss_path: &Path, cache_dir: Option<&Path>) -> Result<Vec<lockfile::Lock>, Error> {
    let mut locks = vec![];

    locks.push(lockfile::acquire(
        moss_path.join(".moss-lockfile"),
        format!("{} another process is using the moss root", "Blocking".yellow().bold()),
    )?);

    if let Some(path) = cache_dir {
        locks.push(lockfile::acquire(
            path.join(".moss-lockfile"),
            format!("{} another process is using the cache dir", "Blocking".yellow().bold()),
        )?);
    }

    Ok(locks)
}

/// In older versions of moss, the `/usr` entry was a symlink
/// to an active state. In newer versions, the state is recorded
/// within the installation tree. (`/usr/.stateID`)
fn read_state_id(root: &Path) -> Option<state::Id> {
    let usr_path = root.join("usr");
    let state_path = root.join("usr").join(".stateID");

    if let Some(id) = fs::read_to_string(state_path).ok().and_then(|s| s.parse::<i32>().ok()) {
        return Some(state::Id::from(id));
    } else if let Ok(usr_target) = usr_path.read_link() {
        return read_legacy_state_id(&usr_target);
    }

    None
}

// Legacy `/usr` link support
fn read_legacy_state_id(usr_target: &Path) -> Option<state::Id> {
    if usr_target.ends_with("usr") {
        let parent = usr_target.parent()?;
        let base = parent.file_name()?;
        let id = base.to_str()?.parse::<i32>().ok()?;

        return Some(state::Id::from(id));
    }

    None
}

/// Ensures moss directories are created
fn ensure_dirs_exist(root: &Path) {
    let moss = root.join(".moss");

    for path in [
        moss.join("db"),
        moss.join("cache"),
        moss.join("assets"),
        moss.join("repo"),
        moss.join("root").join("staging"),
        moss.join("root").join("isolation"),
    ] {
        let _ = fs::create_dir_all(path);
    }
    ensure_cachedir_tag(&moss.join("cache"));
}

/// Ensure we install a cachedir tag to prevent backup tools
/// from archiving the contents of this tree.
fn ensure_cachedir_tag(path: &Path) {
    let cachedir_tag = path.join("CACHEDIR.TAG");
    if !cachedir_tag.exists() {
        let _ = fs::write(
            cachedir_tag,
            br#"Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by moss.
# For information about cache directory tags see https://bford.info/cachedir/"#,
        );
    }
}

/// Errors specific to a target installation filesystem
#[derive(Debug, Error)]
pub enum Error {
    #[error("Root is invalid")]
    RootInvalid,
    #[error("Cache dir is invalid")]
    CacheInvalid,
    #[error("acquiring lockfile")]
    Lockfile(#[from] lockfile::Error),
}
