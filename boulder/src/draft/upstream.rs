use std::{
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};

use fs_err::tokio::{self as fs, File};
use futures_util::{StreamExt, TryStreamExt, stream};
use moss::{environment, request, runtime, util};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command};
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};
use url::Url;

use crate::Env;

pub struct Upstream {
    pub uri: Url,
    pub hash: String,
}

/// Fetch and extract the provided upstreams under `extract_root`
pub fn fetch_and_extract(env: &Env, upstreams: &[Url], extract_root: &Path) -> Result<Vec<Upstream>, Error> {
    let mpb = MultiProgress::new();

    let ret = runtime::block_on(
        stream::iter(upstreams)
            .map(|uri| async {
                let (temp_file, temp_path) = NamedTempFile::with_prefix("boulder-")?.into_parts();
                let archive_file = File::from_std(fs_err::File::from_parts(temp_file, &temp_path));

                let pb = mpb.add(
                    ProgressBar::new_spinner()
                        .with_style(
                            ProgressStyle::with_template(" {spinner} {wide_msg}")
                                .unwrap()
                                .tick_chars("--=≡■≡=--"),
                        )
                        .with_message(format!("{} {}", "Downloading".blue(), *uri)),
                );
                pb.enable_steady_tick(Duration::from_millis(150));

                let hash = fetch(uri, archive_file).await?;

                // Hardlink or copy fetched asset to cache dir so we don't need
                // to refetch it when the user finally builds this new recipe
                {
                    let cache_path = fetched_upstream_cache_path(env, uri, &hash);

                    if let Some(parent) = cache_path.parent() {
                        fs::create_dir_all(parent).await?;
                    }

                    util::async_hardlink_or_copy(&temp_path, &cache_path).await?;
                }

                pb.set_message(format!("{} {}", "Extracting".yellow(), *uri));

                extract(&temp_path, extract_root).await?;

                // Cleanup temp path
                drop(temp_path);

                pb.suspend(|| println!("{} {}", "Fetched".green(), *uri));

                Ok(Upstream { uri: uri.clone(), hash })
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect(),
    );

    println!();

    ret
}

async fn fetch(url: &Url, mut file: File) -> Result<String, Error> {
    let mut stream = request::stream(url.clone()).await?;

    let mut hasher = Sha256::new();

    while let Some(bytes) = stream.next().await {
        let mut bytes = bytes?;
        hasher.update(&bytes);
        file.write_all_buf(&mut bytes).await?;
    }

    file.flush().await?;

    let hash = hex::encode(hasher.finalize());

    Ok(hash)
}

async fn extract(archive: &Path, destination: &Path) -> Result<(), Error> {
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

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to run `bsdtar`")]
    Bsdtar(#[source] io::Error),
    #[error("failed to infer file type of `{path}`")]
    InferFileType { path: PathBuf, source: io::Error },
    #[error("io")]
    Io(#[from] io::Error),
    #[error("request")]
    Request(#[from] request::Error),
    #[error("extract failed with code {0}")]
    Extract(ExitStatus),
}
