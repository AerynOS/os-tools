// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;
use std::{path::Path, time::Duration};

use fs_err::tokio::{self as fs};
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::{environment, runtime, util};
use sha2::{Digest, Sha256};
use stone_recipe::upstream::Kind as SourceKind;
use stone_recipe::upstream::SourceUri;
use tempfile::NamedTempFile;

use tui::{MultiProgress, ProgressBar, Styled};
use url::Url;

use crate::Env;
use crate::upstream::{Error, Upstream, git, plain};

/// Fetches a list of upstreams and stores them into the cache directory on success.
pub fn fetch(env: &Env, upstreams: &[SourceUri]) -> Result<Vec<Upstream>, Error> {
    let mpb = MultiProgress::new();

    let ret = runtime::block_on(
        stream::iter(upstreams)
            .map(|uri| async {
                let pb = mpb.add(ProgressBar::new_spinner());
                pb.enable_steady_tick(Duration::from_millis(150));
                let upstream_dir = env.cache_dir.join("upstreams");

                let upstream = match &uri.kind {
                    SourceKind::Archive => fetch_archive(&uri.clone(), &upstream_dir, &pb).await?,
                    SourceKind::Git => fetch_git_repo(&uri.clone(), &upstream_dir, &pb).await?,
                };

                pb.suspend(|| println!("{} {}", "Fetched".green(), uri.url));

                Ok(upstream)
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect(),
    );

    println!();

    ret
}

/// Extracts the upstream into the destination directory.
/// The destination directory must exist.
pub fn extract(env: &Env, upstream: &Upstream, extract_root: &Path) -> Result<(), Error> {
    let upstream_container_dir = env.cache_dir.join("upstreams");
    let upstream_dir = upstream.stored_path(&upstream_container_dir);
    runtime::block_on(async {
        match upstream {
            Upstream::Plain(_) => plain::extract(&upstream_dir, extract_root).await.map_err(Error::from),
            Upstream::Git(_) => git::clone_to(&upstream_dir, extract_root)
                .await
                .map_err(|e| Error::from(git::Error::Git(e))),
        }
    })?;
    Ok(())
}

pub fn fetched_upstream_cache_path(env: &Env, uri: &Url, hash: &str) -> PathBuf {
    // FIXME: contrary to the other functions here,
    // this function *knows* too much, because it was left intact after a refactor
    // (as it was out of the scope of the refactor).
    // It should eventually be stripped as well.

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

async fn fetch_archive(uri: &SourceUri, upstreams_dir: &Path, pb: &ProgressBar) -> Result<Upstream, Error> {
    let temp_path = NamedTempFile::with_prefix("boulder-")?.into_temp_path();

    let hash = plain::fetch(uri.url.clone(), &temp_path, pb).await?;
    let archive = plain::Plain {
        url: uri.url.clone(),
        hash,
        rename: None,
    };

    let final_path = archive.stored_path(upstreams_dir);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    util::async_hardlink_or_copy(&temp_path, &final_path).await?;

    Ok(Upstream::Plain(archive))
}

async fn fetch_git_repo(uri: &SourceUri, upstreams_dir: &Path, pb: &ProgressBar) -> Result<Upstream, Error> {
    let mut git_upstream = git::Git {
        url: uri.url.clone(),
        commit: "HEAD".to_owned(),
        original_index: 0,
    };
    let final_path = git_upstream.stored_path(upstreams_dir);

    // git::clone_mirror() will fail if the path exists, but
    // fetch_archive() happily overwrites final_path if it exists:
    // replicate the same behavior here.
    util::remove_dir_all(&final_path)?;

    let repo = git::clone_mirror(&uri.url, &git_upstream.stored_path(upstreams_dir), pb)
        .await
        .map_err(|e| Error::from(git::Error::Git(e)))?;

    git_upstream.commit = repo
        .peel_commit(&git_upstream.commit)
        .await
        .map_err(|e| Error::from(git::Error::Git(e)))?;

    Ok(Upstream::Git(git_upstream))
}
