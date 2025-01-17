use std::{io, path::Path, process::ExitStatus, time::Duration};

use fs_err::tokio::{self as fs, File};
use futures_util::{stream, StreamExt, TryStreamExt};
use moss::{environment, request, runtime};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command};
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};
use url::Url;

use crate::util;

pub struct Upstream {
    pub uri: Url,
    pub hash: String,
}

/// Fetch and extract the provided upstreams under `extract_root`
pub fn fetch_and_extract(upstreams: &[Url], extract_root: &Path) -> Result<Vec<Upstream>, Error> {
    util::recreate_dir(extract_root)?;

    let mpb = MultiProgress::new();

    let ret = runtime::block_on(
        stream::iter(upstreams)
            .map(|uri| async {
                let name = util::uri_file_name(uri);
                let archive_path = extract_root.join(name);

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

                let hash = fetch(uri, &archive_path).await?;

                pb.set_message(format!("{} {}", "Extracting".yellow(), *uri));

                extract(&archive_path, extract_root).await?;

                fs::remove_file(archive_path).await?;

                pb.suspend(|| println!("{} {}", "Fetched".green(), *uri));

                Ok(Upstream { uri: uri.clone(), hash })
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect(),
    );

    println!();

    ret
}

async fn fetch(url: &Url, output: &Path) -> Result<String, Error> {
    let mut stream = request::get(url.clone()).await?;

    let mut file = File::create(&output).await?;

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
    let extension = archive
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "tar".to_owned());

    // If we can't specialise (.zip, etc) assume its a tar
    let result = match extension.as_str() {
        "zip" => {
            Command::new("unzip")
                .arg(archive)
                .arg("-d")
                .arg(destination)
                .output()
                .await?
        }
        _ => {
            Command::new("tar")
                .arg("xf")
                .arg(archive)
                .arg("-C")
                .arg(destination)
                .output()
                .await?
        }
    };

    if result.status.success() {
        Ok(())
    } else {
        eprintln!("Command exited with: {}", String::from_utf8_lossy(&result.stderr));
        Err(Error::Extract(result.status))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),
    #[error("request")]
    Request(#[from] request::Error),
    #[error("extract failed with code {0}")]
    Extract(ExitStatus),
}
