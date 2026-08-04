// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! Drivers for creating portable filesystem trees (`fstree`) from _virtual_
//! fstrees ([`vfs::Tree`]) and their backing content (`CAS` / content address store).

use std::{fmt, path::Path};

use astr::AStr;
use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};

use crate::{Installation, package};

pub use self::native::NativeDriver;

pub mod native;

/// A specific `fstree` format supported by `moss`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Format {
    /// An `fstree` backed by the native filesystem, using
    /// reflinks, hardlinks or normal copy operations to
    /// populate based on the best strategy.
    Native,
}

/// A driver capable of managing the lifecycle of an `fstree` for a specific [`Format`].
pub trait Driver {
    /// Driver specific error
    type Error;

    /// Blit a new `fstree` to `target` from the supplied virtual fstree
    /// and asset backing from [`Installation`].
    fn blit(
        &self,
        installation: &Installation,
        tree: &vfs::Tree<PendingFile>,
        target: &Path,
    ) -> Result<(), Self::Error>;

    /// Bring up an `fstree` at the `target` path with the requsted `Mutability`.
    ///
    /// Some types of fstrees require mounting to be active & usable. That happens
    /// at this layer, if needed.
    fn bring_up(&self, _target: &Path, _mutability: Mutability) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Bring down an `fstree` at the `target` path.
    ///
    /// Some types of fstrees require unmounting to be disabled. That happens
    /// at this layer, if needed.
    fn bring_down(&self, _target: &Path) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The requested mutability of an `fstree`
pub enum Mutability {
    /// Read only
    ReadOnly,
    /// Read write
    ReadWrite,
}

/// A file pending creation to an `fstree`
#[derive(Debug, Clone)]
pub struct PendingFile {
    /// The origin package for this file/inode
    pub id: package::Id,

    /// Corresponding layout entry, describing the inode
    pub layout: StonePayloadLayoutRecord,
}

impl vfs::BlitFile for PendingFile {
    /// Match internal kind to minimalist vfs kind
    fn kind(&self) -> vfs::tree::Kind {
        match &self.layout.file {
            StonePayloadLayoutFile::Symlink(source, _) => vfs::tree::Kind::Symlink(source.clone()),
            StonePayloadLayoutFile::Directory(_) => vfs::tree::Kind::Directory,
            _ => vfs::tree::Kind::Regular,
        }
    }

    /// Return ID for conflict
    fn id(&self) -> AStr {
        self.id.clone().into()
    }

    /// Resolve the target path, including the missing `/usr` prefix
    fn path(&self) -> AStr {
        let result = match &self.layout.file {
            StonePayloadLayoutFile::Regular(_, target) => target.clone(),
            StonePayloadLayoutFile::Symlink(_, target) => target.clone(),
            StonePayloadLayoutFile::Directory(target) => target.clone(),
            StonePayloadLayoutFile::CharacterDevice(target) => target.clone(),
            StonePayloadLayoutFile::BlockDevice(target) => target.clone(),
            StonePayloadLayoutFile::Fifo(target) => target.clone(),
            StonePayloadLayoutFile::Socket(target) => target.clone(),
            StonePayloadLayoutFile::Unknown(.., target) => target.clone(),
        };

        vfs::path::join("/usr", &result)
    }

    /// Clone the node to a reparented path, for symlink resolution
    fn cloned_to(&self, path: AStr) -> Self {
        let mut new = self.clone();
        new.layout.file = match &self.layout.file {
            StonePayloadLayoutFile::Regular(source, _) => StonePayloadLayoutFile::Regular(*source, path),
            StonePayloadLayoutFile::Symlink(source, _) => StonePayloadLayoutFile::Symlink(source.clone(), path),
            StonePayloadLayoutFile::Directory(_) => StonePayloadLayoutFile::Directory(path),
            StonePayloadLayoutFile::CharacterDevice(_) => StonePayloadLayoutFile::CharacterDevice(path),
            StonePayloadLayoutFile::BlockDevice(_) => StonePayloadLayoutFile::BlockDevice(path),
            StonePayloadLayoutFile::Fifo(_) => StonePayloadLayoutFile::Fifo(path),
            StonePayloadLayoutFile::Socket(_) => StonePayloadLayoutFile::Socket(path),
            StonePayloadLayoutFile::Unknown(source, _) => StonePayloadLayoutFile::Unknown(source.clone(), path),
        };
        new
    }
}

impl From<AStr> for PendingFile {
    fn from(value: AStr) -> Self {
        PendingFile {
            id: Default::default(),
            layout: StonePayloadLayoutRecord {
                uid: 0,
                gid: 0,
                mode: 0o755,
                tag: 0,
                file: StonePayloadLayoutFile::Directory(value),
            },
        }
    }
}

impl fmt::Display for PendingFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as vfs::BlitFile>::path(self).fmt(f)
    }
}

impl AsRef<StonePayloadLayoutRecord> for PendingFile {
    fn as_ref(&self) -> &StonePayloadLayoutRecord {
        &self.layout
    }
}

/// Build a [`vfs::Tree`] for the specified layouts.
///
/// Returns a newly built [`vfs::Tree`] that can be used in
/// the creation of fstrees.
pub fn vfs(layouts: Vec<(package::Id, StonePayloadLayoutRecord)>) -> Result<vfs::Tree<PendingFile>, vfs::tree::Error> {
    let mut tbuild = vfs::TreeBuilder::new();

    for (id, layout) in layouts {
        tbuild.push(PendingFile { id: id.clone(), layout });
    }

    tbuild.bake();
    tbuild.tree()
}
