//! Git repository manipulation utilities based
//! on the `git` executable.
//!
//! For any operation, `git` is called under the hood:
//! make sure it is available in your `$PATH`, otherwise
//! [Error] will be returned.
//!
//! Even though we are aware that calling executables is brittle API,
//! neither libgit2 nor gitoxide had all operations available in this
//! module implemented.

use std::ffi::OsStr;
use std::path::{self, Path, PathBuf};
use std::process::Stdio;

use tokio::{io, process};
use url::Url;

pub mod error;
pub use self::error::Error;
use error::{Constraint, InnerError};

/// A Git repository.
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    /// Opens a local bare Git repository.
    /// If the Git repository at `path` is not bare,
    /// an [Error] containing [Constraint::NotBare] is returned.
    pub async fn open_bare(path: &Path) -> Result<Self, Error> {
        let path = path::absolute(path).map_err(InnerError::from)?;
        let output = run_git(&[
            OsStr::new("-C"),
            path.as_os_str(),
            OsStr::new("repo"),
            OsStr::new("info"),
            OsStr::new("layout.bare"),
        ])
        .await?;
        if !output.stdout.starts_with(b"layout.bare=true") {
            return Err(InnerError::Constraint(Constraint::NotBare))?;
        }
        Ok(Self { path })
    }

    /// Clones a local or remote Git repository as bare into `path`.
    /// The clone is performed with Git's `--mirror` flag.
    pub async fn clone_mirror(path: &Path, url: &Url) -> Result<Self, Error> {
        let path = path::absolute(path).map_err(InnerError::from)?;
        run_git(&[
            OsStr::new("clone"),
            OsStr::new("--mirror"),
            OsStr::new(&url.as_str()),
            path.as_os_str(),
        ])
        .await?;
        Ok(Self { path })
    }

    /// Clones a local or remote Git repository as bare into `path`.
    /// The clone is performed with Git's `--mirror` flag.
    /// A callback is fired repeatedly to track the cloning
    /// process in real time.
    pub async fn clone_mirror_progress<F>(path: &Path, url: &Url, callback: F) -> Result<Self, Error>
    where
        F: Fn(FetchProgress),
    {
        let path = path::absolute(path).map_err(InnerError::from)?;
        run_git_progress(
            &[
                OsStr::new("clone"),
                OsStr::new("--mirror"),
                OsStr::new("--progress"),
                OsStr::new(&url.as_str()),
                path.as_os_str(),
            ],
            callback,
        )
        .await?;
        Ok(Self { path })
    }

    /// Whether this repository has a commit identified by its hash.
    pub async fn has_commit(&self, commit: &str) -> Result<bool, Error> {
        let output = run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("cat-file"),
            OsStr::new("-t"),
            OsStr::new(commit),
        ])
        .await?;
        Ok(output.stdout.starts_with(b"commit"))
    }

    /// Returns the remote URL for the provided `remote`
    pub async fn get_remote_url(&self, remote: &str) -> Result<String, Error> {
        let output = run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("remote"),
            OsStr::new("get-url"),
            OsStr::new(remote),
        ])
        .await?;
        Ok(str::from_utf8(&output.stdout).unwrap_or("").to_owned())
    }

    /// Sets the remote URL for the provided `remote` to `url`
    pub async fn set_remote_url(&self, remote: &str, url: &str) -> Result<(), Error> {
        run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("remote"),
            OsStr::new("set-url"),
            OsStr::new(remote),
            OsStr::new(url),
        ])
        .await?;
        Ok(())
    }

    /// Checkout the provided `rev` (branch or commit)
    pub async fn checkout(&self, rev: &str) -> Result<(), Error> {
        run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("checkout"),
            OsStr::new(rev),
        ])
        .await?;
        Ok(())
    }

    /// Equivalent to `git fetch`.
    /// A callback is fired repeatedly to track the fetching
    /// process in real time.
    pub async fn fetch_progress<F>(&self, callback: F) -> Result<(), Error>
    where
        F: Fn(FetchProgress),
    {
        run_git_progress(
            &[
                OsStr::new("-C"),
                self.path.as_os_str(),
                OsStr::new("fetch"),
                OsStr::new("--progress"),
            ],
            callback,
        )
        .await?;
        Ok(())
    }

    /// Clone the current [`Repository`] to the provided `path` and return
    /// the cloned to [`Repository`].
    pub async fn clone_to(&self, path: &Path) -> Result<Self, Error> {
        let path = path::absolute(path).map_err(InnerError::from)?;

        // Clone it to `path`
        run_git(&[OsStr::new("clone"), self.path.as_os_str(), path.as_os_str()]).await?;

        Ok(Self { path: path.to_owned() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The argument of callbacks when they are invoked
/// for reporting a Git operation's progress.
pub struct FetchProgress {
    /// Completion percentage.
    pub percent: u8,
    /// Download speed in formatted human units per second
    pub speed: String,
}

/// Runs git and waits for it to terminate.
async fn run_git<I, S>(args: I) -> Result<std::process::Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = process::Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(InnerError::from)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(InnerError::Run {
            code: output.status.code(),
            stderr: Some(String::from_utf8(output.stderr).unwrap()),
        })?
    }
}

async fn run_git_progress<I, S, F>(args: I, callback: F) -> Result<(), Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    F: Fn(FetchProgress),
{
    let (mut git, stderr) = spawn_git(args)?;

    let parser = async move {
        let prog = ProgressParser::new(stderr);
        prog.parse(callback).await
    };

    let (_, result) = tokio::join!(parser, git.wait());
    let result = result.map_err(InnerError::from)?;
    if result.success() {
        Ok(())
    } else {
        Err(InnerError::Run {
            code: result.code(),
            stderr: None,
        })?
    }
}

fn spawn_git<I, S>(args: I) -> Result<(process::Child, process::ChildStderr), Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = process::Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(InnerError::from)?;
    let stderr = child.stderr.take().unwrap();
    Ok((child, stderr))
}

struct ProgressParser<R: io::AsyncRead> {
    reader: io::BufReader<R>,
}

impl<R: io::AsyncRead + Unpin> ProgressParser<R> {
    const TERMINATOR: u8 = b'\r';
    const PREFIX: &[u8] = b"Receiving objects:";

    pub fn new(stderr: R) -> Self {
        Self {
            reader: io::BufReader::new(stderr),
        }
    }

    // We're parsing lines like:
    // "Receiving objects:  26% (163045/627093), 52.57 MiB | 34.99 MiB/s"
    // And we want the percentage and the speed, which are conveniently
    // the first and the last tokens of the line.

    pub async fn parse(self, callback: impl Fn(FetchProgress)) -> Result<(), Error> {
        use tokio::io::AsyncBufReadExt;

        let mut lines = self.reader.split(Self::TERMINATOR);
        while let Some(line) = lines.next_segment().await.map_err(InnerError::from)? {
            if !line.starts_with(Self::PREFIX) {
                continue;
            }
            let line = &str::from_utf8(&line[Self::PREFIX.len()..]).unwrap_or("");
            if let Some(progress) = Self::parse_progress(line) {
                callback(progress);
            }
        }
        Ok(())
    }

    fn parse_progress(line: &str) -> Option<FetchProgress> {
        let mut tokens = line.split_ascii_whitespace();

        let percent = tokens.next()?;
        let unit_per_sec = tokens.next_back()?;
        let speed = tokens.next_back()?;

        if !unit_per_sec.ends_with("/s") {
            return None;
        }

        Some(FetchProgress {
            percent: percent.strip_suffix('%')?.parse().ok()?,
            speed: format!("{speed} {unit_per_sec}"),
        })
    }
}
