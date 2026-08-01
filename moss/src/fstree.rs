// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! Drivers for creating portable filesystem trees (`fstree`) from _virtual_
//! fstrees ([`vfs::Tree`]) and their backing content (`CAS` / content address store).

use std::path::PathBuf;
use std::{fmt, path::Path};

use astr::AStr;
use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};
use thiserror::Error;

use crate::{Installation, package};

pub use self::native::NativeDriver;
pub use self::overlayimg::OverlayimgDriver;

pub mod native;
pub mod overlayimg;

/// A specific `fstree` format supported by `moss`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Format {
    /// An `fstree` backed by the native filesystem, using
    /// reflinks, hardlinks or normal copy operations to
    /// populate based on the best strategy.
    Native,
    /// An `fstree` backed by an EROFS meta-only image and
    /// overlay mount to provide deduplicated content and
    /// per file metadata.
    Overlayimg,
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
    fn bring_up(
        &self,
        _installation: &Installation,
        _target: &Path,
        _mutability: Mutability,
    ) -> Result<(), Self::Error> {
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

/// An error from a driver
#[derive(Debug, Error)]
pub enum DriverError {
    #[error("native fstree driver")]
    Native(native::Error),
    #[error("overlayimg fstree driver")]
    Overlayimg(overlayimg::Error),
}

/// A type erased [`Driver`]
pub struct AnyDriver {
    inner: Box<dyn Driver<Error = DriverError> + Send + Sync + 'static>,
}

impl AnyDriver {
    fn new<T: Driver + Send + Sync + 'static>(inner: T, f: fn(T::Error) -> DriverError) -> Self {
        struct Adapter<T: Driver> {
            inner: T,
            f: fn(T::Error) -> DriverError,
        }

        impl<T: Driver> Adapter<T> {
            fn new(inner: T, f: fn(T::Error) -> DriverError) -> Self {
                Self { inner, f }
            }
        }

        impl<T: Driver> Driver for Adapter<T> {
            type Error = DriverError;

            fn blit(
                &self,
                installation: &Installation,
                tree: &vfs::Tree<PendingFile>,
                target: &Path,
            ) -> Result<(), Self::Error> {
                self.inner.blit(installation, tree, target).map_err(self.f)
            }

            fn bring_up(
                &self,
                installation: &Installation,
                target: &Path,
                mutability: Mutability,
            ) -> Result<(), Self::Error> {
                self.inner.bring_up(installation, target, mutability).map_err(self.f)
            }

            fn bring_down(&self, target: &Path) -> Result<(), Self::Error> {
                self.inner.bring_down(target).map_err(self.f)
            }
        }

        Self {
            inner: Box::new(Adapter::new(inner, f)),
        }
    }

    /// Create an erased native driver
    pub fn native() -> Self {
        Self::new(NativeDriver, DriverError::Native)
    }

    /// Create an erased overlayimg driver
    pub fn overlayimg(driver: OverlayimgDriver) -> Self {
        Self::new(driver, DriverError::Overlayimg)
    }

    /// Blit a new `fstree` to `target` from the supplied virtual fstree
    /// and asset backing from [`Installation`].
    pub fn blit<'a>(
        &'a self,
        installation: &'a Installation,
        tree: vfs::Tree<PendingFile>,
        target: PathBuf,
    ) -> Result<Fstree<'a>, DriverError> {
        self.inner.blit(installation, &tree, &target)?;

        Ok(Fstree {
            driver: self,
            installation,
            vfs: tree,
            path: target,
            status: Status::Down,
        })
    }
}

impl Driver for AnyDriver {
    type Error = DriverError;

    fn blit(
        &self,
        installation: &Installation,
        tree: &vfs::Tree<PendingFile>,
        target: &Path,
    ) -> Result<(), Self::Error> {
        self.inner.blit(installation, tree, target)
    }

    fn bring_up(&self, installation: &Installation, target: &Path, mutability: Mutability) -> Result<(), Self::Error> {
        self.inner.bring_up(installation, target, mutability)
    }

    fn bring_down(&self, target: &Path) -> Result<(), Self::Error> {
        self.inner.bring_down(target)
    }
}

/// Handle to an `fstree`
pub struct Fstree<'a> {
    driver: &'a AnyDriver,
    installation: &'a Installation,
    /// VFS used to create this `fstree`
    pub vfs: vfs::Tree<PendingFile>,
    /// Path to this `fstree`
    pub path: PathBuf,
    /// Stateful status of this `fstree`
    pub status: Status,
}

impl Fstree<'_> {
    /// Bring up this `fstree` with the requsted [`Mutability`].
    pub fn bring_up(&mut self, mutability: Mutability) -> Result<(), DriverError> {
        self.driver.bring_up(self.installation, &self.path, mutability)?;
        self.status = Status::Up { mutability };
        Ok(())
    }

    /// Bring down this `fstree`.
    pub fn bring_down(&mut self) -> Result<(), DriverError> {
        self.driver.bring_down(&self.path)?;
        self.status = Status::Down;
        Ok(())
    }

    /// Change the mutability of an fstree.
    ///
    /// Returns `true` if the operation was applied.
    ///
    /// Returns `false` if the fstree was already at this mutability or
    /// if the fstree is currently [`Status::Down`].
    pub fn change_mutability(&mut self, new_mutability: Mutability) -> Result<bool, DriverError> {
        match self.status {
            Status::Up { mutability } if mutability != new_mutability => {
                self.driver.bring_down(&self.path)?;
                self.driver.bring_up(self.installation, &self.path, new_mutability)?;
                self.status = Status::Up {
                    mutability: new_mutability,
                };
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Stateful status of an fstree.
pub enum Status {
    /// Fstree is down, if applicable.
    Down,
    /// Fstree is up, if applicable.
    ///
    /// See [`Driver::bring_up`].
    Up {
        /// Mutability of the fstree
        mutability: Mutability,
    },
}

/// The requested mutability of an `fstree`
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
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
