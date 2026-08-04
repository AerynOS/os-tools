// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err::{self as fs, File, os::unix::fs::symlink};
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use snafu::{ResultExt, Snafu, ensure_whatever, whatever};

use crate::{Installation, util};

use super::{Driver, Mutability, PendingFile};

pub use erofs::XattrNamespace;

#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayimgDriver {
    erofs_image_writer: erofs::MetaImageWriter,
}

impl OverlayimgDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_xattr_namespace(self, xattr_namespace: XattrNamespace) -> Self {
        Self {
            erofs_image_writer: self.erofs_image_writer.with_xattr_namespace(xattr_namespace),
        }
    }
}

impl Driver for OverlayimgDriver {
    type Error = Error;

    fn blit(&self, installation: &Installation, tree: &vfs::Tree<PendingFile>, target: &Path) -> Result<(), Error> {
        self.blit(installation, tree, target)
            .with_whatever_context(|_| format!("blit fstree to {}", target.display()))
    }

    fn bring_up(&self, installation: &Installation, target: &Path, mutability: Mutability) -> Result<(), Error> {
        self.bring_up(installation, target, mutability)
            .with_whatever_context(|_| format!("bring up {mutability} fstree at {}", target.display()))
    }

    fn bring_down(&self, target: &Path) -> Result<(), Error> {
        self.bring_down(target)
            .with_whatever_context(|_| format!("bring down fstree at {}", target.display()))
    }
}

impl OverlayimgDriver {
    fn blit(&self, _installation: &Installation, tree: &vfs::Tree<PendingFile>, target: &Path) -> Result<(), Error> {
        // Constucts all paths
        let paths = Paths::new(target);

        // Scaffold the new fstree
        self.scaffold(tree, &paths).whatever_context("scaffold new fstree")?;

        // Write an EROFS image to the designated path
        let mut erofs_image = File::create_new(&paths.erofs_image).whatever_context("create erofs.img file")?;
        self.erofs_image_writer
            .write(tree, &mut erofs_image)
            .whatever_context("write erofs.img to file")?;

        // That's everything! The real magic happens during `enable`
        Ok(())
    }

    fn bring_up(&self, installation: &Installation, target: &Path, mutability: Mutability) -> Result<(), Error> {
        // Constucts all paths
        let paths = Paths::new(target);

        // Ensure we have a valid overlayimg fstree
        self.assert_is_valid_fstree(&paths)
            .whatever_context("check if fstree is valid")?;

        // Mount
        self.mount(installation, mutability, &paths)
            .whatever_context("mount the fstree")?;

        Ok(())
    }

    fn bring_down(&self, target: &Path) -> Result<(), Error> {
        // Constucts all paths
        let paths = Paths::new(target);

        // Unmount
        self.unmount(&paths).whatever_context("unmount the fstree")?;

        Ok(())
    }

    /// Scaffolds a new `fstree` at `target`
    fn scaffold(&self, tree: &vfs::Tree<PendingFile>, paths: &Paths) -> Result<(), Error> {
        let scaffold_dirs = || -> io::Result<_> {
            // Recreate the fstree
            util::recreate_dir(&paths.root)?;
            // Create all dirs
            fs::create_dir_all(&paths.usr_fstree)?;
            fs::create_dir(&paths.erofs)?;
            fs::create_dir(&paths.extra)?;
            fs::create_dir(&paths.work)?;
            fs::create_dir(&paths.merged)?;
            Ok(())
        };

        scaffold_dirs().whatever_context("scaffold dirs")?;

        // Create links from all root `/usr/` entries
        // into `merged` where they will actually live
        // after we `enable` this fstree (where the overlay
        // will be mounted)
        let Some(vfs::tree::Element::Directory(_, _, entries)) = tree.structured_from("/usr") else {
            whatever!("vfs is missing /usr to construct fstree from");
        };
        for entry in entries {
            let name = entry.file_name();
            // Relative to `usr/`
            let source = Path::new(".fstree").join("merged/usr").join(name);
            let target = paths.root.join("usr").join(name);
            symlink(source, target).whatever_context("symlink into .fstree/merged")?;
        }

        Ok(())
    }

    fn assert_is_valid_fstree(&self, paths: &Paths) -> Result<(), Error> {
        let assert_exists = |path: &Path| {
            let relpath = path.strip_prefix(&paths.root).unwrap_or(path);
            ensure_whatever!(path.exists(), "{} is missing", relpath.display());
            Ok(())
        };

        assert_exists(&paths.usr_fstree)?;
        assert_exists(&paths.erofs_image)?;
        assert_exists(&paths.erofs)?;
        assert_exists(&paths.extra)?;
        assert_exists(&paths.work)?;
        assert_exists(&paths.merged)?;

        Ok(())
    }

    fn mount(&self, installation: &Installation, mutability: Mutability, paths: &Paths) -> Result<(), Error> {
        // Mount EROFS
        mount(
            Some(&paths.erofs_image),
            &paths.erofs,
            Some("erofs"),
            MsFlags::empty(),
            Some(""),
        )
        .whatever_context("mount erofs.img")?;

        let overlay_options = match mutability {
            Mutability::ReadOnly => format!(
                "lowerdir={}:{}::{}",
                paths.extra.display(),
                paths.erofs.display(),
                installation.assets_path("v2").display(),
            ),
            Mutability::ReadWrite => format!(
                "lowerdir={}::{},upperdir={},workdir={}",
                paths.erofs.display(),
                installation.assets_path("v2").display(),
                paths.extra.display(),
                paths.work.display()
            ),
        };

        // Mount overlay
        mount(
            Some("overlay"),
            &paths.merged,
            Some("overlay"),
            MsFlags::empty(),
            Some(overlay_options.as_str()),
        )
        .whatever_context("mount overlay")?;

        Ok(())
    }

    fn unmount(&self, paths: &Paths) -> Result<(), Error> {
        // Unmount overlay
        umount2(&paths.merged.canonicalize().unwrap(), MntFlags::MNT_DETACH).whatever_context("unmount overlay")?;
        // Unmount erofs
        umount2(&paths.erofs.canonicalize().unwrap(), MntFlags::MNT_DETACH).whatever_context("unmount erofs")?;
        Ok(())
    }
}

/// Required paths used by an overlayimg fstree
struct Paths {
    /// Root `/` of the fstree
    root: PathBuf,
    /// overlayimg fstrees require state & mountpoints
    /// to become enabled. That all lives under here so
    /// it can be atomically shuttled w/ `/usr`.
    usr_fstree: PathBuf,
    /// Path we will write the EROFS image to
    erofs_image: PathBuf,
    /// Where we mount the erofs.img
    erofs: PathBuf,
    /// Overlay folder used as an upper layer when
    /// [`Mutability::ReadWrite`] and used as the
    /// first lower layer when [`Mutability::ReadOnly`]
    ///
    /// This is where things like triggers & other extra
    /// files will live that aren't part of the immutable
    /// EROFS base image.
    extra: PathBuf,
    /// Overlay work dir used when [`Mutability::ReadWrite`]
    work: PathBuf,
    /// Overlay merged mount dir that holds the final fstree
    /// and will be linked into from `/usr`
    merged: PathBuf,
}

impl Paths {
    fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let usr_fstree = root.join("usr/.fstree");
        let erofs_image = usr_fstree.join("erofs.img");
        let erofs = usr_fstree.join("erofs");
        let extra = usr_fstree.join("extra");
        let work = usr_fstree.join("work");
        let merged = usr_fstree.join("merged");

        Self {
            root,
            usr_fstree,
            erofs_image,
            erofs,
            extra,
            work,
            merged,
        }
    }
}

#[derive(Debug, Snafu)]
#[snafu(whatever, display("{message}"))]
pub struct Error {
    message: String,
    #[snafu(source(from(Box<dyn std::error::Error + Send + Sync + 'static>, Some)))]
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}
