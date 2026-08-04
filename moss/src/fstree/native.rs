// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    env, io,
    os::fd::{AsRawFd, FromRawFd, RawFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs_err as fs;
use nix::{
    NixPath,
    errno::Errno,
    fcntl::{self, AtFlags, OFlag},
    libc::{AT_FDCWD, FICLONE, ioctl},
    sys::stat::{Mode, fchmod, fstat, mkdirat, umask},
    unistd::{Gid, Uid, close, fchown, mkdir, symlinkat},
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use stone::StonePayloadLayoutFile;
use thiserror::Error;
use tracing::{debug, info, trace, warn};
use tui::{ProgressBar, ProgressStyle, Styled};
use vfs::tree::{BlitFile, Element};

use crate::Installation;

use super::{Driver, PendingFile};

#[derive(Debug, Clone, Copy)]
pub struct NativeDriver;

impl Driver for NativeDriver {
    type Error = Error;

    fn blit(&self, installation: &Installation, tree: &vfs::Tree<PendingFile>, target: &Path) -> Result<(), Error> {
        blit_root(installation, tree, target)
    }
}

struct BlitContext<'a> {
    is_user_root: bool,
    blit_strategy: BlitStrategy,
    cache_fd: RawFd,
    progress: &'a ProgressBar,
}

#[derive(Debug, Clone, Copy, Default)]
struct BlitStats {
    num_files: u64,
    num_symlinks: u64,
    num_dirs: u64,
}

impl BlitStats {
    fn merge(self, other: Self) -> Self {
        Self {
            num_files: self.num_files + other.num_files,
            num_symlinks: self.num_symlinks + other.num_symlinks,
            num_dirs: self.num_dirs + other.num_dirs,
        }
    }

    fn num_entries(&self) -> u64 {
        self.num_files + self.num_symlinks + self.num_dirs
    }
}

#[derive(Debug, Clone, Copy, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
enum BlitStrategy {
    Reflink,
    Hardlink,
    Copy,
}

/// Attempts to find the best blit strategy in order of priority: `reflink` -> `hardlink` -> `copy`
///
/// `copy` is only used if `EXDEV` / `cross-device link not permitted`, otherwise
/// any other error during this test falls back to `hardlink` to ensure we don't
/// accidentally `copy` on unexpected errors, and to introduce regression against
/// the previous `hardlink` only behavior
///
/// If `MOSS_FSTREE_NATIVE_STRATEGY` env var is set, that is used instead of the
/// above identification logic.
fn identify_blit_strategy(installation: &Installation, blit_target: &Path) -> BlitStrategy {
    // Allow overriding via env var
    if let Some(strategy) = env::var("MOSS_FSTREE_NATIVE_STRATEGY")
        .ok()
        .and_then(|s| s.parse::<BlitStrategy>().ok())
    {
        return strategy;
    }

    let from = installation.cache_path(".link-test");
    let to = blit_target.join(".link-test");

    let identify = || -> io::Result<_> {
        // Remove both if they already exist
        if from.exists() {
            fs::remove_file(&from)?;
        }
        if to.exists() {
            fs::remove_file(&to)?;
        }

        // Create empty from
        fs::write(&from, b"")?;

        // Open from file
        let source = fs::OpenOptions::new().read(true).open(&from)?;
        // Target file should be _new_ for reflink test
        let target = fs::OpenOptions::new().write(true).create_new(true).open(&to)?;

        // Attempt reflink
        let res = reflink(source, target);

        if res.is_ok() {
            return Ok(BlitStrategy::Reflink);
        }

        // Remove to created above so we can test hardlink
        fs::remove_file(&to)?;

        // Attempt hardlink
        let res = linkat(None, &from, None, &to);

        match res {
            Err(Errno::EXDEV) => Ok(BlitStrategy::Copy),
            _ => Ok(BlitStrategy::Hardlink),
        }
    };

    // Fallback to hardlink on error
    let strategy = identify().unwrap_or_else(|err| {
        warn!(
            error = format!("{err:#}"),
            "Failed to identify blit strategy, falling back to hardlink strategy"
        );
        BlitStrategy::Hardlink
    });

    // Cleanup
    let _ = fs::remove_file(&from);
    let _ = fs::remove_file(&to);

    strategy
}

/// Queries the current hard limit for number of open files
/// and sets it as the soft limit.
fn set_nofile_ulimit_soft_to_hard() {
    use nix::sys::resource::{Resource, getrlimit, setrlimit};

    let (soft_limit, hard_limit) = match getrlimit(Resource::RLIMIT_NOFILE) {
        Ok(limits) => limits,
        Err(error) => {
            warn!(%error,"Failed to call getrlimit(nofile)");
            return;
        }
    };

    debug!(%soft_limit, %hard_limit, "Queried current process nofile limits");

    if let Err(error) = setrlimit(Resource::RLIMIT_NOFILE, hard_limit, hard_limit) {
        warn!(%error, "Failed to call setrlimit(nofile, {hard_limit}, {hard_limit})");
        return;
    }

    info!(%hard_limit, "Updated current process nofile soft limit to hard limit");
}

/// Blit the packages to a filesystem root
///
/// This functionality is core to all moss filesystem transactions, forming the entire
/// staging logic. For all the [`crate::package::Id`] present in the staging state,
/// query their stored [`StonePayloadLayoutBody`] and cache into a [`vfs::Tree`].
///
/// The new `/usr` filesystem is written in optimal order to a staging tree by making
/// use of the "at" family of functions (`mkdirat`, `linkat`, etc) with relative directory
/// file descriptors, linking files from the assets store to provide deduplication.
///
/// This provides a very quick means to generate a hardlinked "snapshot" on-demand,
/// which can then be activated via [`Self::promote_staging`]
pub fn blit_root(installation: &Installation, tree: &vfs::Tree<PendingFile>, blit_target: &Path) -> Result<(), Error> {
    // Up the process nofile soft limit to be equal
    // to the allowed hard limit to minimize the risk
    // of EMFILE during blit
    set_nofile_ulimit_soft_to_hard();

    let is_user_root = Uid::effective().is_root();

    // Recreate dir
    let _ = fs::remove_dir_all(blit_target);
    mkdir(blit_target, Mode::from_bits_truncate(0o755))?;

    // Ensure umask is cleared so we get exact modes
    let old_umask = umask(Mode::empty());

    // Identify blit strategy based on FS in use
    let blit_strategy = identify_blit_strategy(installation, blit_target);
    info!(%blit_strategy, "Blit strategy identified");

    let progress = ProgressBar::new(1).with_style(
        ProgressStyle::with_template("\n|{bar:20.red/blue}| {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("■≡=- "),
    );
    progress.set_message("Blitting filesystem");
    progress.enable_steady_tick(Duration::from_millis(150));
    progress.tick();

    let now = Instant::now();
    let mut stats = BlitStats::default();

    progress.set_length(tree.len());
    progress.set_position(0_u64);

    let cache_dir = installation.assets_path("v2");
    let cache_fd = fcntl::open(&cache_dir, OFlag::O_DIRECTORY | OFlag::O_RDONLY, Mode::empty())?;

    let ctx = BlitContext {
        is_user_root,
        blit_strategy,
        cache_fd,
        progress: &progress,
    };

    // We need to ensure this runtime is dropped so it doesn't linger
    // since this is in the boulder call path & boulder can't have
    // multithreading when CLONE into a user namespace / "container"
    let rayon_runtime = rayon::ThreadPoolBuilder::new().build().expect("rayon runtime");

    rayon_runtime.install(|| -> Result<(), Error> {
        if let Some(root) = tree.structured() {
            let root_dir = fcntl::open(blit_target, OFlag::O_DIRECTORY | OFlag::O_RDONLY, Mode::empty())?;

            if let Element::Directory(_, _, children) = root {
                let current_span = tracing::Span::current();
                stats = stats.merge(
                    children
                        .into_par_iter()
                        .map(|child| {
                            let _guard = current_span.enter();
                            blit_element(&ctx, root_dir, child)
                        })
                        .try_reduce(BlitStats::default, |a, b| Ok(a.merge(b)))?,
                );
            }

            close(root_dir)?;
        }

        Ok(())
    })?;

    // Cleanup
    close(ctx.cache_fd)?;
    umask(old_umask);
    progress.finish_and_clear();

    let elapsed = now.elapsed();
    let num_entries = stats.num_entries();

    println!(
        "\n{} entries blitted in {} {}",
        num_entries.to_string().bold(),
        format!("{:.2}s", elapsed.as_secs_f32()).bold(),
        format!("({:.1}k / s)", num_entries as f32 / elapsed.as_secs_f32() / 1_000.0).dim()
    );

    Ok(())
}

/// Recursively write a directory, or a single flat inode, to the staging tree.
/// Care is taken to retain the directory file descriptor to avoid costly path
/// resolution at runtime.
fn blit_element(ctx: &BlitContext<'_>, parent: RawFd, element: Element<'_, PendingFile>) -> Result<BlitStats, Error> {
    let mut stats = BlitStats::default();

    ctx.progress.inc(1);

    let (_, item) = match &element {
        Element::Directory(_, item, _) => ("directory", item),
        Element::Child(_, item) => ("file", item),
    };

    trace!(
        progress = ctx.progress.position() as f32 / ctx.progress.length().unwrap_or(1) as f32,
        current = ctx.progress.position() as usize,
        total = ctx.progress.length().unwrap_or(0) as usize,
        event_type = "progress_update",
        "Blitting {}",
        item.path()
    );

    match element {
        Element::Directory(name, item, children) => {
            // Construct within the parent
            blit_element_item(ctx, parent, name, item, &mut stats)?;

            // open the new dir
            let newdir = fcntl::openat(parent, name, OFlag::O_RDONLY | OFlag::O_DIRECTORY, Mode::empty())?;

            let current_span = tracing::Span::current();
            stats = stats.merge(
                children
                    .into_par_iter()
                    .map(|child| {
                        let _guard = current_span.enter();
                        blit_element(ctx, newdir, child)
                    })
                    .try_reduce(BlitStats::default, |a, b| Ok(a.merge(b)))?,
            );

            close(newdir)?;

            Ok(stats)
        }
        Element::Child(name, item) => {
            blit_element_item(ctx, parent, name, item, &mut stats)?;

            Ok(stats)
        }
    }
}

/// Write a single inode into the staging tree.
///
/// # Arguments
///
/// * `parent`  - raw file descriptor for parent directory in which the inode is being record to
/// * `cache`   - raw file descriptor for the system asset pool tree
/// * `subpath` - the base name of the new inode
/// * `item`    - New inode being recorded
fn blit_element_item(
    ctx: &BlitContext<'_>,
    parent: RawFd,
    subpath: &str,
    item: &PendingFile,
    stats: &mut BlitStats,
) -> Result<(), Error> {
    const EMPTY_FILE_HASH: u128 = 0x99aa_06d3_0147_98d8_6001_c324_468d_497f;

    match &item.layout.file {
        StonePayloadLayoutFile::Regular(id, _) => {
            let hash = format!("{id:02x}");
            let directory = if hash.len() >= 10 {
                PathBuf::from(&hash[..2]).join(&hash[2..4]).join(&hash[4..6])
            } else {
                "".into()
            };

            // Link relative from cache to target
            let fp = directory.join(hash);

            // Strip write bit.
            //
            // Our blitted roots are intended for read-only, immutable use.
            // This doesn't necessarily help enforce anything and we will
            // leverage other strategies to enforce true immutability. But
            // no write bit is ever needed, so may as well strip it.
            let blit_mode = item.layout.mode & !0o222;

            match *id {
                // Mystery empty-file hash. Do not allow dupes!
                // https://github.com/AerynOS/os-tools/issues/372
                EMPTY_FILE_HASH => {
                    let fd = fcntl::openat(
                        parent,
                        subpath,
                        OFlag::O_CREAT | OFlag::O_WRONLY | OFlag::O_TRUNC,
                        // We can specify mode here used to create the file.
                        Mode::from_bits_truncate(blit_mode),
                    )?;
                    close(fd)?;
                }
                // Regular file
                _ => {
                    match ctx.blit_strategy {
                        BlitStrategy::Reflink => {
                            // Open FD to source
                            let src = fcntl::openat(ctx.cache_fd, &fp, OFlag::O_RDONLY, Mode::empty())?;
                            // Create new inode & get its FD.
                            let dest = fcntl::openat(
                                parent,
                                subpath,
                                OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY,
                                // We can specify mode here used to create the file.
                                Mode::from_bits_truncate(blit_mode),
                            )?;

                            // Apply special bits stripped from mode during `openat`
                            if blit_mode & 0o7000 > 0 {
                                fchmod(dest, Mode::from_bits_truncate(blit_mode))?;
                            }

                            // Reflink from source to dest
                            reflink(src, dest)?;

                            // Update ownership if we can
                            if ctx.is_user_root {
                                let uid = Uid::from_raw(item.layout.uid);
                                let gid = Gid::from_raw(item.layout.gid);

                                if !(uid.is_root() || gid.as_raw() == 0) {
                                    fchown(
                                        dest,
                                        Some(Uid::from_raw(item.layout.uid)),
                                        Some(Gid::from_raw(item.layout.gid)),
                                    )?;
                                }
                            }

                            // Cleanup FDs
                            close(src)?;
                            close(dest)?;
                        }
                        BlitStrategy::Hardlink => {
                            // Open FD for CAS file
                            let src = fcntl::openat(ctx.cache_fd, &fp, OFlag::O_RDONLY, Mode::empty())?;
                            // Get existing mode
                            let existing_mode = fstat(src)?.st_mode;

                            // Handle permissions
                            //
                            // Since hardlinks share inode / metadata, this is a
                            // dangerous operation as any changes here impact ALL
                            // links to this file and may impact the running system.
                            // We need to ensure mode can never lose its executability
                            // if another file needs that executability.
                            //
                            // We therefore combine the two (more permissive) and always
                            // strip out the `w` bit since this should be immutable / read-only.
                            // If that is different from the existing mode, we can safely apply it.
                            let desired_mode = (existing_mode | blit_mode) & !0o222;

                            if existing_mode != desired_mode {
                                fchmod(src, Mode::from_bits_truncate(desired_mode))?;
                            }

                            // Create link to blit target
                            linkat(Some(src), "", Some(parent), subpath)?;

                            // Cleanup FD
                            close(src)?;
                        }
                        #[allow(clippy::disallowed_types)]
                        BlitStrategy::Copy => {
                            // Open FD to source
                            let src = fcntl::openat(ctx.cache_fd, &fp, OFlag::O_RDONLY, Mode::empty())?;
                            // Create dest inode & get its FD.
                            let dest = fcntl::openat(
                                parent,
                                subpath,
                                OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY,
                                // We can specify mode here used to create the file.
                                Mode::from_bits_truncate(blit_mode),
                            )?;

                            // Apply special bits stripped from mode during `openat`
                            if blit_mode & 0o7000 > 0 {
                                fchmod(dest, Mode::from_bits_truncate(blit_mode))?;
                            }

                            // Copy (rust uses copy_file_range where possible)
                            //
                            // FDs are dropped when File is dropped
                            let mut from = unsafe { std::fs::File::from_raw_fd(src) };
                            let mut to = unsafe { std::fs::File::from_raw_fd(dest) };
                            io::copy(&mut from, &mut to).map_err(|_| Errno::last())?;

                            // Update ownership if we can
                            if ctx.is_user_root {
                                let uid = Uid::from_raw(item.layout.uid);
                                let gid = Gid::from_raw(item.layout.gid);

                                if !(uid.is_root() || gid.as_raw() == 0) {
                                    fchown(
                                        dest,
                                        Some(Uid::from_raw(item.layout.uid)),
                                        Some(Gid::from_raw(item.layout.gid)),
                                    )?;
                                }
                            }
                        }
                    }
                }
            }

            stats.num_files += 1;
        }
        StonePayloadLayoutFile::Symlink(source, _) => {
            symlinkat(source.as_str(), Some(parent), subpath)?;
            stats.num_symlinks += 1;
        }
        StonePayloadLayoutFile::Directory(_) => {
            mkdirat(parent, subpath, Mode::from_bits_truncate(item.layout.mode))?;
            stats.num_dirs += 1;
        }

        // Unimplemented
        StonePayloadLayoutFile::CharacterDevice(_)
        | StonePayloadLayoutFile::BlockDevice(_)
        | StonePayloadLayoutFile::Fifo(_)
        | StonePayloadLayoutFile::Socket(_)
        | StonePayloadLayoutFile::Unknown(..) => {}
    };

    Ok(())
}

/// [`nix::unistd::linkat`] doesn't supply AT_EMPTY_PATH
/// to allow `olddirfd` to be the source FD, so we override
/// their implementation to support this usecase
fn linkat<P: ?Sized + NixPath>(
    olddirfd: Option<RawFd>,
    oldpath: &P,
    newdirfd: Option<RawFd>,
    newpath: &P,
) -> nix::Result<()> {
    use nix::libc;

    let atflag = if olddirfd.is_some() && oldpath.is_empty() {
        AtFlags::AT_EMPTY_PATH
    } else {
        AtFlags::empty()
    };
    let at_rawfd = |fd: Option<RawFd>| match fd {
        Some(fd) => fd,
        None => AT_FDCWD,
    };

    let res = oldpath.with_nix_path(|oldcstr| {
        newpath.with_nix_path(|newcstr| unsafe {
            libc::linkat(
                at_rawfd(olddirfd),
                oldcstr.as_ptr(),
                at_rawfd(newdirfd),
                newcstr.as_ptr(),
                atflag.bits(),
            )
        })
    })??;
    Errno::result(res).map(drop)
}

fn reflink(source: impl AsRawFd, target: impl AsRawFd) -> nix::Result<()> {
    let res = unsafe { ioctl(target.as_raw_fd(), FICLONE, source.as_raw_fd()) };
    Errno::result(res).map(drop)
}

#[derive(Debug, Error)]
pub enum Error {
    // TODO: actual errors!!
    #[error("blit")]
    Blit(#[from] Errno),
}
