use std::ffi::OsStr;
use std::fmt::Debug;
use std::path::{self, Path, PathBuf};
use std::process::Stdio;
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
        let path = path::absolute(path)?;
        let output = run_git(&[
            OsStr::new("-C"),
            path.as_os_str(),
            OsStr::new("repo"),
            OsStr::new("info"),
            OsStr::new("layout.bare"),
        ])
        .await?;
        if !output.stdout.starts_with(b"layout.bare=true") {
            return Err(Error::NotBare);
        }
        Ok(Self { path })
    }

    /// Clones a local or remote Git repository as bare into `path`.
    pub async fn clone_bare(path: &Path, url: &Url) -> Result<Self, Error> {
        let path = path::absolute(path)?;
        run_git(&[
            OsStr::new("clone"),
            OsStr::new("--mirror"),
            OsStr::new(&url.as_str()),
            path.as_os_str(),
        ])
        .await?;
        Ok(Self { path })
    }

    pub async fn clone_bare_progress<F>(path: &Path, url: &Url, callback: F) -> Result<Self, Error>
    where
        F: Fn(FetchProgress),
    {
        let path = path::absolute(path)?;
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

    pub async fn has_commit(&self, commit: &str) -> Result<bool, Error> {
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

    pub async fn add_worktree(&self, path: &Path, commit: &str) -> Result<Worktree, Error> {
        let path = path::absolute(path)?;

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

pub struct Worktree {
    repo: PathBuf,
    worktree: PathBuf,
}

impl Worktree {
    pub async fn remove(&self) -> Result<(), Error> {
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
                "unexpectedly".to_owned()
            },
        msg=
            if let Some(msg) = _1 {
                 format!(". Diagnostic output below:\n{msg}")
                } else {
                     "".to_owned()
            }
    )]
    Run(Option<i32>, Option<String>),
    #[error("this repository is not bare")]
    NotBare,
}

pub struct FetchProgress {
    pub percent: u8,
    /// Download speed in bytes per second.
    pub speed: u64,
}

/// Runs git and waits for it to terminate.
/// When no error occured, a dump of stderr is returned.
async fn run_git<I, S>(args: I) -> Result<std::process::Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = process::Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(output)
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
        .stderr(Stdio::piped())
        .spawn()?;
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
        while let Some(line) = lines.next_segment().await? {
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
