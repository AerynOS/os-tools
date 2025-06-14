// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use fs_err as fs;
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::runtime;
use nix::unistd::{LinkatFlags, linkat};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};
use url::Url;

use crate::{Paths, Recipe, util};

/// Cache all upstreams from the provided [`Recipe`] and make them available
/// in the guest rootfs.
pub fn sync(recipe: &Recipe, paths: &Paths) -> Result<(), Error> {
    let upstreams = recipe
        .parsed
        .upstreams
        .iter()
        .cloned()
        .map(Upstream::from_recipe)
        .collect::<Result<Vec<_>, _>>()?;

    println!();
    println!("Sharing {} upstream(s) with the build container", upstreams.len());

    let mp = MultiProgress::new();
    let tp = mp.add(
        ProgressBar::new(upstreams.len() as u64).with_style(
            ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
                .unwrap()
                .progress_chars("■≡=- "),
        ),
    );
    tp.tick();

    let upstream_dir = paths.guest_host_path(&paths.upstreams());
    util::ensure_dir_exists(&upstream_dir)?;

    runtime::block_on(
        stream::iter(&upstreams)
            .map(|upstream| async {
                let pb = mp.insert_before(
                    &tp,
                    ProgressBar::new(u64::MAX).with_message(format!(
                        "{} {}",
                        "Downloading".blue(),
                        upstream.name().bold(),
                    )),
                );
                pb.enable_steady_tick(Duration::from_millis(150));

                let install = upstream.fetch(paths, &pb).await?;

                pb.set_message(format!("{} {}", "Copying".yellow(), upstream.name().bold()));
                pb.set_style(
                    ProgressStyle::with_template(" {spinner} {wide_msg} ")
                        .unwrap()
                        .tick_chars("--=≡■≡=--"),
                );

                runtime::unblock({
                    let install = install.clone();
                    let dir = upstream_dir.clone();
                    move || install.share(&dir)
                })
                .await?;

                let cached_tag = install
                    .was_cached()
                    .then_some(format!("{}", " (cached)".dim()))
                    .unwrap_or_default();

                pb.finish();
                mp.remove(&pb);
                mp.suspend(|| println!("{} {}{cached_tag}", "Shared".green(), upstream.name().bold()));
                tp.inc(1);

                Ok(()) as Result<_, Error>
            })
            .buffer_unordered(moss::environment::MAX_NETWORK_CONCURRENCY)
            .try_collect::<()>(),
    )?;

    mp.clear()?;
    println!();

    Ok(())
}

#[derive(Clone)]
enum Installed {
    Plain {
        name: String,
        path: PathBuf,
        was_cached: bool,
    },
    Git {
        name: String,
        path: PathBuf,
        was_cached: bool,
    },
}

impl Installed {
    fn was_cached(&self) -> bool {
        match self {
            Installed::Plain { was_cached, .. } => *was_cached,
            Installed::Git { was_cached, .. } => *was_cached,
        }
    }

    fn share(&self, dest_dir: &Path) -> Result<(), Error> {
        match self {
            Installed::Plain { name, path, .. } => {
                let target = dest_dir.join(name);

                // Attempt hard link
                let link_result = linkat(None, path, None, &target, LinkatFlags::NoSymlinkFollow);

                // Copy instead
                if link_result.is_err() {
                    fs::copy(path, &target)?;
                }
            }
            Installed::Git { name, path, .. } => {
                let target = dest_dir.join(name);
                util::copy_dir(path, &target)?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Upstream {
    Plain(Plain),
    Git(Git),
}

impl Upstream {
    pub fn from_recipe(upstream: stone_recipe::Upstream) -> Result<Self, Error> {
        match upstream {
            stone_recipe::Upstream::Plain { uri, hash, rename, .. } => Ok(Self::Plain(Plain {
                uri,
                hash: hash.parse()?,
                rename,
            })),
            stone_recipe::Upstream::Git {
                uri, ref_id, staging, ..
            } => Ok(Self::Git(Git { uri, ref_id, staging })),
        }
    }

    fn name(&self) -> &str {
        match self {
            Upstream::Plain(plain) => plain.name(),
            Upstream::Git(git) => git.name(),
        }
    }

    async fn fetch(&self, paths: &Paths, pb: &ProgressBar) -> Result<Installed, Error> {
        match self {
            Upstream::Plain(plain) => plain.fetch(paths, pb).await,
            Upstream::Git(git) => git.fetch(paths, pb).await,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hash(String);

impl FromStr for Hash {
    type Err = ParseHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < 5 {
            return Err(ParseHashError::TooShort(s.to_owned()));
        }

        Ok(Self(s.to_owned()))
    }
}

#[derive(Debug, Error)]
pub enum ParseHashError {
    #[error("hash too short: {0}")]
    TooShort(String),
}

#[derive(Debug, Clone)]
pub struct Plain {
    uri: Url,
    hash: Hash,
    rename: Option<String>,
}

impl Plain {
    fn name(&self) -> &str {
        if let Some(name) = &self.rename {
            name
        } else {
            util::uri_file_name(&self.uri)
        }
    }

    fn path(&self, paths: &Paths) -> PathBuf {
        // Hash uri and file hash together
        // for a unique file path that can
        // be used for caching purposes and
        // is busted if either uri or hash
        // change
        let mut hasher = Sha256::new();
        hasher.update(self.uri.as_str());
        hasher.update(&self.hash.0);

        let hash = hex::encode(hasher.finalize());

        paths
            .upstreams()
            .host
            .join("fetched")
            // Type safe guaranteed to be >= 5 bytes
            .join(&hash[..5])
            .join(&hash[hash.len() - 5..])
            .join(hash)
    }

    async fn fetch(&self, paths: &Paths, pb: &ProgressBar) -> Result<Installed, Error> {
        use moss::request;
        use tokio::fs;

        pb.set_style(
            ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
                .unwrap()
                .tick_chars("--=≡■≡=--"),
        );

        let name = self.name();
        let path = self.path(paths);
        let partial_path = path.with_extension("part");

        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }

        if path.exists() {
            return Ok(Installed::Plain {
                name: name.to_owned(),
                path,
                was_cached: true,
            });
        }

        let mut stream = request::get(self.uri.clone()).await?;

        let mut hasher = Sha256::new();
        let mut out = fs::File::create(&partial_path).await?;

        while let Some(chunk) = stream.next().await {
            let bytes = &chunk?;
            pb.inc(bytes.len() as u64);
            hasher.update(bytes);
            out.write_all(bytes).await?;
        }

        out.flush().await?;

        let hash = hex::encode(hasher.finalize());

        if hash != self.hash.0 {
            fs::remove_file(&partial_path).await?;

            return Err(Error::HashMismatch {
                name: name.to_owned(),
                expected: self.hash.0.clone(),
                got: hash,
            });
        }

        fs::rename(partial_path, &path).await?;

        Ok(Installed::Plain {
            name: name.to_owned(),
            path,
            was_cached: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Git {
    uri: Url,
    ref_id: String,
    staging: bool,
}

impl Git {
    fn name(&self) -> &str {
        util::uri_file_name(&self.uri)
    }

    fn final_path(&self, paths: &Paths) -> PathBuf {
        paths
            .upstreams()
            .host
            .join("git")
            .join(util::uri_relative_path(&self.uri))
    }

    fn staging_path(&self, paths: &Paths) -> PathBuf {
        paths
            .upstreams()
            .host
            .join("staging")
            .join("git")
            .join(util::uri_relative_path(&self.uri))
    }

    async fn fetch(&self, paths: &Paths, pb: &ProgressBar) -> Result<Installed, Error> {
        use tokio::fs;

        pb.set_style(
            ProgressStyle::with_template(" {spinner} {wide_msg} ")
                .unwrap()
                .tick_chars("--=≡■≡=--"),
        );

        let clone_path = if self.staging {
            self.staging_path(paths)
        } else {
            self.final_path(paths)
        };
        let clone_path_string = clone_path.display().to_string();

        let final_path = self.final_path(paths);
        let final_path_string = final_path.display().to_string();

        if let Some(parent) = clone_path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }
        if let Some(parent) = final_path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }

        if self.ref_exists(&final_path).await? {
            self.reset_to_ref(&final_path).await?;
            return Ok(Installed::Git {
                name: self.name().to_owned(),
                path: final_path,
                was_cached: true,
            });
        }

        let _ = fs::remove_dir_all(&clone_path).await;
        if self.staging {
            let _ = fs::remove_dir_all(&final_path).await;
        }

        let mut args = vec!["clone"];
        if self.staging {
            args.push("--mirror");
        }
        args.extend(["--", self.uri.as_str(), &clone_path_string]);

        self.run(&args, None).await?;

        if self.staging {
            self.run(&["clone", "--", &clone_path_string, &final_path_string], None)
                .await?;
        }

        self.reset_to_ref(&final_path).await?;

        Ok(Installed::Git {
            name: self.name().to_owned(),
            path: final_path,
            was_cached: false,
        })
    }

    async fn ref_exists(&self, path: &Path) -> Result<bool, Error> {
        if !path.exists() {
            return Ok(false);
        }

        self.run(&["fetch"], Some(path)).await?;

        let result = self.run(&["cat-file", "-e", &self.ref_id], Some(path)).await;

        Ok(result.is_ok())
    }

    async fn reset_to_ref(&self, path: &Path) -> Result<(), Error> {
        self.run(&["reset", "--hard", &self.ref_id], Some(path)).await?;

        self.run(
            &[
                "submodule",
                "update",
                "--init",
                "--recursive",
                "--depth",
                "1",
                "--jobs",
                "4",
            ],
            Some(path),
        )
        .await?;

        Ok(())
    }

    async fn run(&self, args: &[&str], cwd: Option<&Path>) -> Result<(), Error> {
        use tokio::process;

        let mut command = process::Command::new("git");

        if let Some(dir) = cwd {
            command.current_dir(dir);
        }

        let output = command.args(args).output().await?;

        if !output.status.success() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return Err(Error::GitFailed(self.uri.clone()));
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to clone {0}")]
    GitFailed(Url),
    #[error("parse hash")]
    ParseHash(#[from] ParseHashError),
    #[error("hash mismatch for {name}, expected {expected:?} got {got:?}")]
    HashMismatch {
        name: String,
        expected: String,
        got: String,
    },
    #[error("request")]
    Request(#[from] moss::request::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
