// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::addr_of_mut;
use std::sync::atomic::{AtomicI32, Ordering};

use fs_err::{self as fs, PathExt as _};
use nix::libc::SIGCHLD;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::prctl::set_pdeathsig;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, Signal, kill, sigaction};
use nix::sys::signalfd::SigSet;
use nix::sys::stat::{Mode, umask};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{Pid, Uid, close, pipe, pivot_root, read, sethostname, tcsetpgrp, write};
use thiserror::Error;

use self::idmap::idmap;

mod idmap;

pub struct Container {
    root: PathBuf,
    work_dir: Option<PathBuf>,
    binds: Vec<Bind>,
    networking: bool,
    hostname: Option<String>,
    ignore_host_sigint: bool,
}

impl Container {
    /// Create a new Container using the default options
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            work_dir: None,
            binds: vec![],
            networking: false,
            hostname: None,
            ignore_host_sigint: false,
        }
    }

    /// Override the working directory
    pub fn work_dir(self, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            work_dir: Some(work_dir.into()),
            ..self
        }
    }

    /// Create a read-write bind mount
    pub fn bind_rw(mut self, host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        self.binds.push(Bind {
            source: host.into(),
            target: guest.into(),
            read_only: false,
        });
        self
    }

    /// Create a read-only bind mount
    pub fn bind_ro(mut self, host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        self.binds.push(Bind {
            source: host.into(),
            target: guest.into(),
            read_only: true,
        });
        self
    }

    /// Configure networking availability
    pub fn networking(self, enabled: bool) -> Self {
        Self {
            networking: enabled,
            ..self
        }
    }

    /// Override hostname (via /etc/hostname)
    pub fn hostname(self, hostname: impl ToString) -> Self {
        Self {
            hostname: Some(hostname.to_string()),
            ..self
        }
    }

    /// Ignore `SIGINT` from the parent process. This allows it to be forwarded to a
    /// spawned process inside the container by using [`forward_sigint`].
    pub fn ignore_host_sigint(self, ignore: bool) -> Self {
        Self {
            ignore_host_sigint: ignore,
            ..self
        }
    }

    /// Run `f` as a container process payload
    pub fn run<E>(self, mut f: impl FnMut() -> Result<(), E>) -> Result<(), Error>
    where
        E: std::error::Error + 'static,
    {
        static mut STACK: [u8; 4 * 1024 * 1024] = [0u8; 4 * 1024 * 1024];

        let rootless = !Uid::effective().is_root();

        // Pipe to synchronize parent & child
        let sync = pipe()?;

        let mut flags =
            CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWIPC | CloneFlags::CLONE_NEWUTS;

        if rootless {
            flags |= CloneFlags::CLONE_NEWUSER;
        }

        if !self.networking {
            flags |= CloneFlags::CLONE_NEWNET;
        }

        let clone_cb = Box::new(|| match enter(&self, sync, &mut f) {
            Ok(_) => 0,
            // Write error back to parent process
            Err(error) => {
                let error = format_error(error);
                let mut pos = 0;

                while pos < error.len() {
                    let Ok(len) = write(sync.1, &error.as_bytes()[pos..]) else {
                        break;
                    };

                    pos += len;
                }

                let _ = close(sync.1);

                1
            }
        });
        let pid = unsafe { clone(clone_cb, &mut *addr_of_mut!(STACK), flags, Some(SIGCHLD))? };

        // Update uid / gid map to map current user to root in container
        if rootless {
            idmap(pid)?;
        }

        // Allow child to continue
        write(sync.1, &[Message::Continue as u8])?;
        // Write no longer needed
        close(sync.1)?;

        if self.ignore_host_sigint {
            ignore_sigint()?;
        }

        let status = waitpid(pid, None)?;

        if self.ignore_host_sigint {
            default_sigint()?;
        }

        match status {
            WaitStatus::Exited(_, 0) => Ok(()),
            WaitStatus::Exited(_, _) => {
                let mut error = String::new();
                let mut buffer = [0u8; 1024];

                loop {
                    let len = read(sync.0, &mut buffer)?;

                    if len == 0 {
                        break;
                    }

                    error.push_str(String::from_utf8_lossy(&buffer[..len]).as_ref());
                }

                Err(Error::Failure(error))
            }
            WaitStatus::Signaled(_, signal, _) => Err(Error::Signaled(signal)),
            WaitStatus::Stopped(_, _)
            | WaitStatus::PtraceEvent(_, _, _)
            | WaitStatus::PtraceSyscall(_)
            | WaitStatus::Continued(_)
            | WaitStatus::StillAlive => Err(Error::UnknownExit),
        }
    }
}

/// Reenter the container
fn enter<E>(container: &Container, sync: (i32, i32), mut f: impl FnMut() -> Result<(), E>) -> Result<(), ContainerError>
where
    E: std::error::Error + 'static,
{
    // Ensure process is cleaned up if parent dies
    set_pdeathsig(Signal::SIGKILL).map_err(ContainerError::SetPDeathSig)?;

    // Wait for continue message
    let mut message = [0u8; 1];
    read(sync.0, &mut message).map_err(ContainerError::ReadContinueMsg)?;
    assert_eq!(message[0], Message::Continue as u8);

    // Close unused read end
    close(sync.0).map_err(ContainerError::CloseReadFd)?;

    setup(container)?;

    f().map_err(|e| ContainerError::Run(Box::new(e)))
}

/// Setup the container
fn setup(container: &Container) -> Result<(), ContainerError> {
    if container.networking {
        setup_networking(&container.root)?;
    }

    setup_localhost()?;

    pivot(&container.root, &container.binds)?;

    if let Some(hostname) = &container.hostname {
        sethostname(hostname).map_err(ContainerError::SetHostname)?;
    }

    if let Some(dir) = &container.work_dir {
        set_current_dir(dir)?;
    }

    Ok(())
}

/// Pivot the process into the rootfs
fn pivot(root: &Path, binds: &[Bind]) -> Result<(), ContainerError> {
    const OLD_PATH: &str = "old_root";

    let old_root = root.join(OLD_PATH);

    add_mount(None, "/", None, MsFlags::MS_REC | MsFlags::MS_PRIVATE)?;
    add_mount(Some(root), root, None, MsFlags::MS_BIND)?;

    for bind in binds {
        let source = bind.source.fs_err_canonicalize()?;
        let target = root.join(bind.target.strip_prefix("/").unwrap_or(&bind.target));

        add_mount(Some(&source), &target, None, MsFlags::MS_BIND)?;

        // Remount to enforce readonly flag
        if bind.read_only {
            add_mount(
                Some(source),
                target,
                None,
                MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
            )?;
        }
    }

    ensure_directory(&old_root)?;
    pivot_root(root, &old_root).map_err(ContainerError::PivotRoot)?;

    set_current_dir("/")?;

    add_mount(Some("proc"), "proc", Some("proc"), MsFlags::empty())?;
    add_mount(Some("tmpfs"), "tmp", Some("tmpfs"), MsFlags::empty())?;
    add_mount(
        Some(format!("/{OLD_PATH}/sys").as_str()),
        "sys",
        None,
        MsFlags::MS_BIND | MsFlags::MS_REC | MsFlags::MS_SLAVE,
    )?;
    add_mount(
        Some(format!("/{OLD_PATH}/dev").as_str()),
        "dev",
        None,
        MsFlags::MS_BIND | MsFlags::MS_REC | MsFlags::MS_SLAVE,
    )?;

    umount2(OLD_PATH, MntFlags::MNT_DETACH).map_err(ContainerError::UnmountOldRoot)?;
    fs::remove_dir(OLD_PATH)?;

    umask(Mode::S_IWGRP | Mode::S_IWOTH);

    Ok(())
}

fn setup_networking(root: &Path) -> Result<(), ContainerError> {
    ensure_directory(root.join("etc"))?;
    fs::copy("/etc/resolv.conf", root.join("etc/resolv.conf"))?;
    Ok(())
}

fn setup_localhost() -> Result<(), ContainerError> {
    // TODO: maybe it's better to hunt down the API to do this instead?
    if PathBuf::from("/usr/sbin/ip").exists() {
        Command::new("/usr/sbin/ip")
            .args(["link", "set", "lo", "up"])
            .output()?;
    }
    Ok(())
}

fn ensure_directory(path: impl AsRef<Path>) -> Result<(), ContainerError> {
    let path = path.as_ref();
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

fn add_mount<T: AsRef<Path>>(
    source: Option<T>,
    target: T,
    fs_type: Option<&str>,
    flags: MsFlags,
) -> Result<(), ContainerError> {
    let target = target.as_ref();
    ensure_directory(target)?;
    mount(
        source.as_ref().map(AsRef::as_ref),
        target,
        fs_type,
        flags,
        Option::<&str>::None,
    )
    .map_err(|err| ContainerError::Mount {
        target: target.to_owned(),
        err,
    })?;
    Ok(())
}

fn set_current_dir(path: impl AsRef<Path>) -> io::Result<()> {
    #[derive(Debug, Error)]
    #[error("failed to set current directory to `{}`", path.display())]
    struct SetCurrentDirError {
        source: io::Error,
        path: PathBuf,
    }

    let path = path.as_ref();
    std::env::set_current_dir(path).map_err(|source| {
        io::Error::new(
            source.kind(),
            SetCurrentDirError {
                source,
                path: path.to_owned(),
            },
        )
    })
}

fn ignore_sigint() -> Result<(), nix::Error> {
    let action = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    unsafe { sigaction(Signal::SIGINT, &action)? };
    Ok(())
}

fn default_sigint() -> Result<(), nix::Error> {
    let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    unsafe { sigaction(Signal::SIGINT, &action)? };
    Ok(())
}

pub fn set_term_fg(pgid: Pid) -> Result<(), nix::Error> {
    // Ignore SIGTTOU and get previous handler
    let prev_handler = unsafe {
        sigaction(
            Signal::SIGTTOU,
            &SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty()),
        )?
    };
    // Set term fg to pid
    let res = tcsetpgrp(io::stdin().as_raw_fd(), pgid);
    // Set up old handler
    unsafe { sigaction(Signal::SIGTTOU, &prev_handler)? };

    match res {
        Ok(_) => {}
        // Ignore ENOTTY error
        Err(nix::Error::ENOTTY) => {}
        Err(e) => return Err(e),
    }

    Ok(())
}

/// Forwards `SIGINT` from the current process to the [`Pid`] process
pub fn forward_sigint(pid: Pid) -> Result<(), nix::Error> {
    static PID: AtomicI32 = AtomicI32::new(0);

    PID.store(pid.as_raw(), Ordering::Relaxed);

    extern "C" fn on_int(_: i32) {
        let pid = Pid::from_raw(PID.load(Ordering::Relaxed));
        let _ = kill(pid, Signal::SIGINT);
    }

    let action = SigAction::new(SigHandler::Handler(on_int), SaFlags::empty(), SigSet::empty());
    unsafe { sigaction(Signal::SIGINT, &action)? };

    Ok(())
}

fn format_error(error: impl std::error::Error) -> String {
    let sources = sources(&error);
    sources.join(": ")
}

fn sources(error: &dyn std::error::Error) -> Vec<String> {
    let mut sources = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source.take() {
        sources.push(error.to_string());
        source = error.source();
    }
    sources
}

struct Bind {
    source: PathBuf,
    target: PathBuf,
    read_only: bool,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("exited with failure: {0}")]
    Failure(String),
    #[error("stopped by signal: {}", .0.as_str())]
    Signaled(Signal),
    #[error("unknown exit reason")]
    UnknownExit,
    #[error("error setting up rootless id map")]
    Idmap(#[from] idmap::Error),
    #[error("nix")]
    Nix(#[from] nix::Error),
    #[error("io")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
enum ContainerError {
    #[error(transparent)]
    Run(Box<dyn std::error::Error>),
    #[error("io")]
    Io(#[from] io::Error),

    // Errors from linux system functions
    #[error("set_pdeathsig")]
    SetPDeathSig(#[source] nix::Error),
    #[error("wait for continue message")]
    ReadContinueMsg(#[source] nix::Error),
    #[error("close read end of pipe")]
    CloseReadFd(#[source] nix::Error),
    #[error("sethostname")]
    SetHostname(#[source] nix::Error),
    #[error("pivot_root")]
    PivotRoot(#[source] nix::Error),
    #[error("unmount old root")]
    UnmountOldRoot(#[source] nix::Error),
    #[error("mount {}", target.display())]
    Mount {
        target: PathBuf,
        #[source]
        err: nix::Error,
    },
}

#[repr(u8)]
enum Message {
    Continue = 1,
}
