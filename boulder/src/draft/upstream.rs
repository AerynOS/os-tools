// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;
use std::{io, path::Path, process::ExitStatus, time::Duration};

use fs_err::tokio::{self as fs};
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::{environment, runtime, util};
use sha2::{Digest, Sha256};
use stone_recipe::upstream::SourceUri;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::process::Command;
use tui::{MultiProgress, ProgressBar, Styled};
use url::Url;

use crate::Env;
use crate::upstream::{self, git, plain};

pub struct Upstream {
    pub uri: Url,
    pub hash: String,
}

/// Fetch and extract the provided upstreams under `extract_root`
pub fn fetch_and_extract(env: &Env, upstreams: &[SourceUri], extract_root: &Path) -> Result<Vec<Upstream>, Error> {
    let mpb = MultiProgress::new();

    let ret = runtime::block_on(
        stream::iter(upstreams)
            .map(|uri| async {
                let pb = mpb.add(ProgressBar::new_spinner());
                pb.enable_steady_tick(Duration::from_millis(150));
                let upstream_dir = env.cache_dir.join("upstreams");

                let upstream = match uri.kind {
                    stone_recipe::upstream::Kind::Archive => {
                        fetch_and_extract_archive(&uri.url, &upstream_dir, extract_root, &pb).await?
                    }
                    stone_recipe::upstream::Kind::Git => {
                        fetch_git_repo(&uri.url, &upstream_dir, extract_root, &pb).await?
                    }
                };

                pb.suspend(|| println!("{} {}", "Fetched".green(), *uri));

                Ok(upstream)
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect(),
    );

    println!();

    ret
}

pub fn fetched_upstream_cache_path(env: &Env, uri: &Url, hash: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(uri.as_str());
    hasher.update(hash);

    let hash = hex::encode(hasher.finalize());

    env.cache_dir
        .join("upstreams")
        .join("fetched")
        // Type safe guaranteed to be >= 5 bytes
        .join(&hash[..5])
        .join(&hash[hash.len() - 5..])
        .join(hash)
}

async fn fetch_and_extract_archive(
    url: &Url,
    upstreams_dir: &Path,
    extract_root: &Path,
    pb: &ProgressBar,
) -> Result<Upstream, Error> {
    let temp_path = NamedTempFile::with_prefix("boulder-")?.into_temp_path();

    let hash = plain::fetch(url.clone(), &temp_path, pb)
        .await
        .map_err(upstream::Error::from)?;
    let archive = plain::Plain {
        url: url.clone(),
        hash,
        rename: None,
    };

    // Hardlink or copy fetched asset to cache dir so we don't need
    // to refetch it when the user finally builds this new recipe.
    {
        let final_path = archive.stored_path(upstreams_dir);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        util::async_hardlink_or_copy(&temp_path, &final_path).await?;
    }

    pb.set_message(format!("{} {}", "Extracting".yellow(), *url));
    extract_archive(&temp_path, extract_root).await?;

    Ok(Upstream {
        uri: archive.url,
        hash: archive.hash.to_string(),
    })
}

async fn fetch_git_repo(
    url: &Url,
    upstreams_dir: &Path,
    extract_root: &Path,
    pb: &ProgressBar,
) -> Result<Upstream, Error> {
    let git_upstream = git::Git {
        url: url.clone(),
        commit: "HEAD".to_owned(),
        original_index: 0,
    };
    let repo = git::clone_mirror(url, &git_upstream.stored_path(upstreams_dir), pb)
        .await
        .map_err(|e| upstream::Error::from(git::Error::Git(e)))?;

    // A mirror repository does not write actual source files
    // in the filesystem. A "regular" repository does, so we clone
    // the mirror into extract_root.
    repo.clone_to(extract_root)
        .await
        .map_err(|e| upstream::Error::from(git::Error::Git(e)))?;

    Ok(Upstream {
        uri: git_upstream.url,
        hash: repo
            .peel_commit(&git_upstream.commit)
            .await
            .map_err(|e| upstream::Error::from(git::Error::Git(e)))?
            .to_string(),
    })
}

async fn extract_archive(archive: &Path, destination: &Path) -> Result<(), Error> {
    let result = Command::new("bsdtar")
        .arg("xf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .output()
        .await
        .map_err(Error::Bsdtar)?;
    if result.status.success() {
        Ok(())
    } else {
        eprintln!("Command exited with: {}", String::from_utf8_lossy(&result.stderr));
        Err(Error::Extract(result.status))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to run `bsdtar`")]
    Bsdtar(#[source] io::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Fetch(#[from] upstream::Error),
    #[error("extract failed with code {0}")]
    Extract(ExitStatus),
}
