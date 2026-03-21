//! Git repository manipulation utilities based
//! on the `git` executable.
//!
//! For any operation, `git` is called under the hood:
//! make sure it is available in your `$PATH`, otherwise
//! [RunError::Io] will be returned.
//!
//! Even though we are aware that calling executables is brittle API,
//! neither libgit2 nor gitoxide had all operations available in this
//! module implemented.

use snafu::ResultExt;
use std::ffi::OsStr;
use std::path::{self, Path, PathBuf};
use std::process::Stdio;
use tokio::{
    io,
    process::{self},
};
use url::Url;

pub mod error;
pub use error::*;

/// A Git repository.
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    /// Opens a local bare Git repository.
    /// [Error::NotBare] is returned if it is not a bare repository.
    pub async fn open_bare(path: &Path) -> Result<Self, OpenError> {
        let path = path::absolute(path).context(IoSnafu)?;
        let output = run_git(&[
            OsStr::new("-C"),
            path.as_os_str(),
            OsStr::new("repo"),
            OsStr::new("info"),
            OsStr::new("layout.bare"),
        ])
        .await?;
        if !output.stdout.starts_with(b"layout.bare=true") {
            return Err(OpenError::NotBare);
        }
        Ok(Self { path })
    }

    /// Clones a local or remote Git repository as bare into `path`.
    pub async fn clone_bare(path: &Path, url: &Url) -> Result<Self, RunError> {
        let path = path::absolute(path).context(IoSnafu)?;
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
    /// A callback is fired repeatedly to track the cloning
    /// process in real time.
    pub async fn clone_bare_progress<F>(path: &Path, url: &Url, callback: F) -> Result<Self, RunError>
    where
        F: Fn(FetchProgress),
    {
        let path = path::absolute(path).context(IoSnafu)?;
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
    pub async fn has_commit(&self, commit: &str) -> Result<bool, RunError> {
        let output = run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("cat-file"),
            OsStr::new("-t"),
            OsStr::new(commit),
        ])
        .await?;
        Ok(output.stderr.is_empty())
    }

    /// Equivalent to `git fetch`.
    /// A callback is fired repeatedly to track the fetching
    /// process in real time.
    pub async fn fetch_progress<F>(&self, callback: F) -> Result<(), RunError>
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

    /// Add a new Git worktree at `path`.
    ///
    /// The worktree is checked out at the provided commit.
    /// If a worktree already exists at `path`, is it overwritten.
    ///
    /// This function expects a "peeled" commit hash. If a reference
    /// (e.g. a tag) is passed, [WorktreeError::NotPeeled] is returned.
    /// This ensures the worktree is created with predictable content,
    /// since a reference may change the commit it points to over time.
    pub async fn add_worktree(&self, path: &Path, commit: &str) -> Result<Worktree, WorktreeError> {
        if commit.starts_with("HEAD") {
            return Err(WorktreeError::NotPeeled {
                commit: commit.to_owned(),
            });
        }

        let path = path::absolute(path).context(IoSnafu)?;

        let output = run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("cat-file"),
            OsStr::new("-t"),
            OsStr::new(commit),
        ])
        .await?;
        if !output.stdout.starts_with(b"commit") {
            return Err(WorktreeError::NotPeeled {
                commit: commit.to_owned(),
            });
        }

        run_git(&[
            OsStr::new("-C"),
            self.path.as_os_str(),
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-f"), // Pass double force to overwrite possible locked worktrees.
            OsStr::new("-f"),
            path.as_os_str(),
            OsStr::new(commit),
        ])
        .await?;
        Ok(Worktree {
            repo: self.path.clone(),
            worktree: path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A Git worktree.
pub struct Worktree {
    repo: PathBuf,
    worktree: PathBuf,
}

impl Worktree {
    /// Removes the worktree.
    /// This means removing the actual directory
    /// containing the worktree, and untracking
    /// the worktree from the Git repository.
    pub async fn remove(&self) -> Result<(), RunError> {
        run_git(&[
            OsStr::new("-C"),
            self.repo.as_os_str(),
            OsStr::new("worktree"),
            OsStr::new("remove"),
            self.worktree.as_os_str(),
        ])
        .await
        .map(|_| ())
    }
}

/// The argument of callbacks when they are invoked
/// for reporting a Git operation's progress.
pub struct FetchProgress {
    /// Isn't this self-explanatory?
    pub percent: u8,
    /// Download speed in bytes per second.
    pub speed: u64,
}

/// Runs git and waits for it to terminate.
async fn run_git<I, S>(args: I) -> Result<std::process::Output, RunError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = process::Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .context(IoSnafu)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(RunError::Run {
            code: output.status.code(),
            stderr: Some(String::from_utf8(output.stderr).unwrap()),
        })
    }
}

async fn run_git_progress<I, S, F>(args: I, callback: F) -> Result<(), RunError>
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
    let result = result.context(IoSnafu)?;
    if result.success() {
        Ok(())
    } else {
        Err(RunError::Run {
            code: result.code(),
            stderr: None,
        })
    }
}

fn spawn_git<I, S>(args: I) -> Result<(process::Child, process::ChildStderr), RunError>
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
        .context(IoSnafu)?;
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

    pub async fn parse(self, callback: impl Fn(FetchProgress)) -> Result<(), RunError> {
        use tokio::io::AsyncBufReadExt;

        let mut lines = self.reader.split(Self::TERMINATOR);
        while let Some(line) = lines.next_segment().await.context(IoSnafu)? {
            if !line.starts_with(Self::PREFIX) {
                continue;
            }
            let line = &str::from_utf8(&line[Self::PREFIX.len()..]).unwrap_or("");
            callback(Self::parse_progress(line));
        }
        Ok(())
    }

    fn parse_progress(line: &str) -> FetchProgress {
        let mut tokens = line.split_ascii_whitespace();

        let percent = tokens
            .by_ref()
            .next()
            .map_or("0", |tok| tok.strip_suffix("%").unwrap_or(tok));
        let speed_unit = tokens
            .by_ref()
            .next_back()
            .map_or("B", |tok| tok.strip_suffix("/s").unwrap_or(tok));
        let speed = tokens.by_ref().next_back().unwrap_or("0");

        FetchProgress {
            percent: percent.parse().unwrap_or_default(),
            speed: speed.parse::<f32>().unwrap_or_default().trunc() as u64
                * match speed_unit {
                    "B" => 1,
                    "KiB" => 1 << 10,
                    "MiB" => 1 << 20,
                    "GiB" => 1 << 30,
                    "TiB" => 1 << 40,
                    _ => 1,
                },
        }
    }
}
