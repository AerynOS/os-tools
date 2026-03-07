use std::ffi::OsStr;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::{
    io,
    process::{self},
};
use url::Url;

pub struct Repository {
    path: PathBuf,
}

impl Repository {
    pub async fn open_bare(path: &Path) -> Result<Self, Error> {
        let path = path.canonicalize()?;
        // repo info is a low-overhead
        // Git command to get information about a repository.
        // When passed without arguments, it prints nothing.
        // This is perfect to just validate the existence,
        // and correctness, of a Git repository.
        run_git(&[
            &OsStr::new("-C"),
            path.as_os_str(),
            &OsStr::new("repo"),
            &OsStr::new("info"),
        ])
        .await?;
        Ok(Self { path })
    }

    /// Clones a local or remote Git repository as bare into `path`.
    pub async fn clone_bare(path: &Path, url: &Url) -> Result<Self, Error> {
        let path = path.canonicalize()?;
        run_git(&[
            &OsStr::new("clone"),
            &OsStr::new("--mirror"),
            &OsStr::new(&url.as_str()),
            path.as_os_str(),
        ])
        .await?;
        Ok(Self { path })
    }

    pub async fn clone_bare_progress<F>(path: &Path, url: &Url, callback: F) -> Result<Self, Error>
    where
        F: Fn(FetchProgress),
    {
        let path = path.canonicalize()?;
        run_git_progress(
            &[
                &OsStr::new("clone"),
                &OsStr::new("--mirror"),
                &OsStr::new("--progress"),
                &OsStr::new(&url.as_str()),
                path.as_os_str(),
            ],
            callback,
        )
        .await?;
        Ok(Self { path })
    }

    pub async fn has_commit(&self) -> Result<bool, Error> {
        run_git(&[
            &OsStr::new("clone"),
            &OsStr::new("--mirror"),
            &OsStr::new(&url.as_str()),
            path.as_os_str(),
        ])
        .await
        .ma
    }

    pub async fn fetch_progress<F>(&self, callback: F) -> Result<(), Error>
    where
        F: Fn(FetchProgress),
    {
        run_git_progress(
            &[
                &OsStr::new("-C"),
                self.path.as_os_str(),
                &OsStr::new("fetch"),
                &OsStr::new("--progress"),
            ],
            callback,
        )
        .await?;
        Ok(())
    }

    pub async fn add_worktree(&self, path: PathBuf, commit: &str) -> Result<Worktree, Error> {
        let path = path.canonicalize()?;

        run_git(&[
            &OsStr::new("-C"),
            self.path.as_os_str(),
            &OsStr::new("worktree"),
            &OsStr::new("add"),
            &OsStr::new("-f"), // Pass double force to overwrite possible locked worktrees.
            &OsStr::new("-f"),
            path.as_os_str(),
            &OsStr::new(commit),
        ])
        .await?;
        Ok(Worktree {
            repo: self.path.clone(),
            worktree: path,
        })
    }
}

pub struct Worktree {
    repo: PathBuf,
    worktree: PathBuf,
}

impl Worktree {
    pub async fn remove(&self) -> Result<(), Error> {
        run_git(&[
            &OsStr::new("-C"),
            self.repo.as_os_str(),
            &OsStr::new("worktree"),
            &OsStr::new("remove"),
            self.worktree.as_os_str(),
        ])
        .await
        .map(|_| ())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error(
        "`git` exited {reason}{msg}",
        reason=
            if let Some(code) = _0 {
                format!("with code {code}")
            } else {
                "unexpectedly".to_string()
            },
        msg=
            if let Some(msg) = _1 {
                 format!(". Diagnostic output below:\n{msg}")
                } else {
                     "".to_string()
            }
    )]
    Run(Option<i32>, Option<String>),
}

pub struct FetchProgress {
    pub percent: u8,
    /// Download speed in bytes per second.
    pub speed: u64,
}

/// Runs git and waits for it to terminate.
/// When no error occured, a dump of stderr is returned.
async fn run_git<I, S>(args: I) -> Result<Vec<u8>, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = process::Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        // Tokio's `output()` overrides the stdout setting,
        // but let's keep it for documentation.
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if output.status.success() {
        Ok(output.stderr)
    } else {
        Err(Error::Run(
            output.status.code(),
            Some(String::from_utf8(output.stderr).unwrap()),
        ))
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
    let result = result?;
    if result.success() {
        Ok(())
    } else {
        Err(Error::Run(result.code(), None))
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
        .spawn()?;
    let stderr = child.stderr.take().unwrap();
    Ok((child, stderr))
}

struct ProgressParser<R: io::AsyncRead> {
    reader: io::BufReader<R>,
    buffer: Vec<u8>,
}

impl<R: io::AsyncRead + Unpin> ProgressParser<R> {
    const TERMINATOR: u8 = b'\r';
    const PREFIX: &[u8] = b"Receiving objects:";

    pub fn new(stderr: R) -> Self {
        Self {
            reader: io::BufReader::new(stderr),
            buffer: Vec::new(),
        }
    }

    // We're parsing lines like:
    // "Receiving objects:  26% (163045/627093), 52.57 MiB | 34.99 MiB/s"
    // And we want the percentage and the speed, which are conveniently
    // the first and the last tokens of the line.

    pub async fn parse(mut self, callback: impl Fn(FetchProgress)) -> Result<(), Error> {
        loop {
            let eof = self.reader.read_until(Self::TERMINATOR, &mut self.buffer).await? == 0;
            if eof {
                break;
            }

            if !self.buffer.starts_with(Self::PREFIX) {
                continue;
            }
            let line = &self.buffer[Self::PREFIX.len()..self.buffer.len() - 1];
            let line = &str::from_utf8(line).unwrap_or("");

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
            .rev()
            .next()
            .map_or("B", |tok| tok.strip_suffix("/s").unwrap_or(tok));
        let speed = tokens.by_ref().rev().next().unwrap_or("0");

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
