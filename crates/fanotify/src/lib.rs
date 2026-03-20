// SPDX-FileCopyrightText: Copyright © 2026 Aeryn OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use fs_err::{self as fs};
use std::{
    collections::HashSet,
    ffi::CString,
    io,
    mem::ManuallyDrop,
    os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    path::PathBuf,
    thread,
};

use nix::{
    fcntl::OFlag,
    sys::{
        fanotify::{EventFFlags, Fanotify, FanotifyFidRecord, FanotifyInfoRecord, InitFlags, MarkFlags, MaskFlags},
        stat::Mode,
    },
};

use snafu::{ResultExt, Snafu};

impl Tracker {
    /// Start recording file events on the system
    ///
    /// By default the tracker will record FAN_{CREATE,MODIFY,DELETE} events
    /// on the entire filesystem resolving the filename back to the full path
    /// as the the event type.
    pub fn new() -> Result<Self, Error> {
        let fan_markers = Fanotify::init(
            InitFlags::FAN_CLASS_NOTIF
                | InitFlags::FAN_NONBLOCK
                | InitFlags::FAN_CLOEXEC
                | InitFlags::FAN_UNLIMITED_QUEUE
                | InitFlags::FAN_REPORT_DFID_NAME,
            EventFFlags::O_RDONLY | EventFFlags::O_LARGEFILE,
        )
        .context(InitSnafu)?;

        let dirfd = nix::fcntl::open(
            "/",
            OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .context(InitSnafu)?;

        // TODO: make this configurable? sadly some bitflags are incompatible with each other
        //       and are not well documented
        fan_markers
            .mark(
                MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_FILESYSTEM,
                MaskFlags::FAN_CREATE | MaskFlags::FAN_MODIFY | MaskFlags::FAN_DELETE,
                &dirfd,
                Some("/"),
            )
            .context(MarkSnafu { path: "/" })?;

        let (wake_r, wake_w) = nix::unistd::pipe2(OFlag::O_CLOEXEC | OFlag::O_NONBLOCK).context(PipeSnafu)?;

        let mountpoints = parse_mountpoints();

        let reader = thread::spawn(move || read_loop(fan_markers.as_raw_fd(), wake_r.as_raw_fd(), &mountpoints));

        Ok(Self {
            wake_w,
            reader: Some(reader),
        })
    }

    /// Stop tracking and return collected paths.
    ///
    /// Signals the background thread to do a final drain of any events still
    /// queued in the kernel before returning.
    pub fn finish(mut self) -> Result<HashSet<TrackItem>, Error> {
        let _ = nix::unistd::write(&self.wake_w, &[1u8]);
        self.reader
            .take()
            .unwrap()
            .join()
            .map_err(|_| ThreadPanicSnafu.build())?
            .context(ReaderSnafu)
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = nix::unistd::write(&self.wake_w, &[1u8]);
            let _ = reader.join();
        }
    }
}

fn read_loop(create_fd: RawFd, wake_r_fd: RawFd, mountpoints: &[String]) -> io::Result<HashSet<TrackItem>> {
    let mut result = HashSet::new();

    loop {
        let mut fds = [
            libc::pollfd {
                fd: create_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_r_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }

        let woken = fds[1].revents & libc::POLLIN != 0;

        // Drain before honouring wakeup so events racing with process exit are not lost.
        if fds[0].revents & libc::POLLIN != 0 || woken {
            drain_creates(create_fd, &mut result, mountpoints);
        }

        if woken {
            break;
        }
    }

    Ok(result)
}

/// Wrap a raw fd as a `Fanotify` without taking ownership.
///
/// SAFETY: `fd` must be valid and remain open for the lifetime of the returned
/// value. Use `ManuallyDrop` to prevent the destructor closing the fd.
unsafe fn borrow_fanotify(fd: RawFd) -> ManuallyDrop<Fanotify> {
    ManuallyDrop::new(unsafe { Fanotify::from_owned_fd(OwnedFd::from_raw_fd(fd)) })
}

fn drain_creates(fd: RawFd, out: &mut HashSet<TrackItem>, mountpoints: &[String]) {
    let fan = unsafe { borrow_fanotify(fd) };

    loop {
        // read_events_with_info_records() added in nix PR #2552.
        match fan.read_events_with_info_records() {
            Ok(pairs) if pairs.is_empty() => break,
            Ok(pairs) => {
                for (event, records) in &pairs {
                    for record in records {
                        if let FanotifyInfoRecord::Fid(fid) = record
                            && let Some(path) = resolve_path_from_fid(fid, mountpoints)
                        {
                            out.insert(TrackItem {
                                path,
                                event: event.mask(),
                            });
                        }
                    }
                }
            }
            Err(nix::errno::Errno::EAGAIN) => break,
            Err(_) => break,
        }
    }
}

// TODO: can mounts have spaces? no idea
//fn unescape_mountpoint(s: &str) -> String {
//    let mut out = String::with_capacity(s.len());
//    let mut chars = s.chars().peekable();
//    while let Some(c) = chars.next() {
//        if c == '\\' {
//            // collect 3 octal digits
//            let oct: String = (0..3).filter_map(|_| chars.next()).collect();
//            if let Ok(n) = u8::from_str_radix(&oct, 8) {
//                out.push(n as char);
//                continue;
//            }
//        }
//        out.push(c);
//    }
//    out
//}

fn parse_mountpoints() -> Vec<String> {
    let Ok(mounts) = fs::read_to_string("/proc/self/mounts") else {
        return vec![];
    };

    // Collect all real mountpoints
    let mut mountpoints: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _source = parts.next()?;
            let mountpoint = parts.next()?;
            let fstype = parts.next()?;
            if matches!(
                fstype,
                "proc"
                    | "sysfs"
                    | "devpts"
                    | "cgroup"
                    | "cgroup2"
                    | "devtmpfs"
                    | "hugetlbfs"
                    | "mqueue"
                    | "debugfs"
                    | "tracefs"
                    | "securityfs"
                    | "pstore"
                    | "overlay"
                    | "tmpfs"
            ) {
                return None;
            }
            Some(mountpoint.to_string())
        })
        .collect();

    // Longest mountpoint first
    mountpoints.sort_by_key(|b| std::cmp::Reverse(b.len()));
    mountpoints
}

// FIXME, return Result and handle errors here
fn resolve_path_from_fid(fid: &FanotifyFidRecord, mountpoints: &[String]) -> Option<PathBuf> {
    let name = fid.name()?;
    if name.is_empty() || name == "." {
        return None;
    }

    for mountpoint in mountpoints {
        let Ok(cstr) = CString::new(mountpoint.as_bytes()) else {
            continue;
        };

        let mount_fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if mount_fd < 0 {
            continue;
        }

        let dir_fd = unsafe {
            libc::syscall(
                libc::SYS_open_by_handle_at,
                mount_fd,
                fid.handle().as_ptr() as *const libc::c_void,
                libc::O_PATH | libc::O_CLOEXEC,
            )
        } as i32;

        unsafe { libc::close(mount_fd) };

        if dir_fd < 0 {
            continue;
        }

        let dir_path = fs::read_link(format!("/proc/self/fd/{dir_fd}")).ok();
        unsafe { libc::close(dir_fd) };

        let Some(dir_path) = dir_path else { continue };

        // Verify the resolved path actually lives under this mountpoint.
        // If not, the inode was found on the same block device but belongs
        // to a different mount, keep trying.
        // Note that this implicitly depends on mountpoints being sorted longest first.
        if dir_path.starts_with(mountpoint.as_str()) {
            return Some(dir_path.join(name));
        }
    }

    None
}

#[derive(Hash, Eq, PartialEq, Debug)]
pub struct TrackItem {
    path: PathBuf,
    event: MaskFlags,
}

impl Default for TrackItem {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            event: MaskFlags::empty(),
        }
    }
}

pub struct Tracker {
    wake_w: OwnedFd,
    reader: Option<thread::JoinHandle<io::Result<HashSet<TrackItem>>>>,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("fanotify_init failed: {source}"))]
    Init { source: nix::errno::Errno },
    #[snafu(display("fanotify_mark failed for {path}: {source}"))]
    Mark { path: String, source: nix::errno::Errno },
    #[snafu(display("pipe failed: {source}"))]
    Pipe { source: nix::errno::Errno },
    #[snafu(display("reader thread I/O error: {source}"))]
    Reader { source: io::Error },
    #[snafu(display("reader thread panicked"))]
    ThreadPanic,
    #[snafu(display("nix"))]
    Nix { source: nix::Error },
    #[snafu(display("io"))]
    Io { source: io::Error },
    #[snafu(display("alloc"))]
    NulError { source: std::ffi::NulError },
}
