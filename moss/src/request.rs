// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    io::{self},
    path::{Path, PathBuf},
    pin::Pin,
    sync::OnceLock,
    task,
};

use fs_err::tokio::{self as fs, File};
use futures_util::TryStreamExt;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWriteExt, BufReader};
use url::Url;

use crate::{environment, util::Sha256Wrapper};

/// Shared client for tcp socket reuse and connection limit
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::ClientBuilder::new()
            .referer(false)
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("build reqwest client")
    })
}

/// Downloads a file to the provided path
pub async fn download(url: Url, to: &Path) -> Result<(), Error> {
    let mut reader = fetch(url).await?;

    write_to_file(&mut reader, to).await
}

/// Downloads a file to the provided path & returns it's sha256 hash
pub async fn download_with_sha256(url: Url, to: &Path) -> Result<String, Error> {
    let mut reader = Sha256Wrapper::new(fetch(url).await?);

    write_to_file(&mut reader, to).await?;

    Ok(reader.finalize())
}

/// Downloads a file to the provided path and invokes `on_progress` after each
/// chunk is downloaded
pub async fn download_with_progress(url: Url, to: &Path, on_progress: impl Fn(Progress) + Unpin) -> Result<(), Error> {
    let mut reader = ProgressRead::new(fetch(url).await?, on_progress);

    write_to_file(&mut reader, to).await
}

/// Downloads a file to the provided path, invokes `on_progress` after each
/// chunk is downloaded and returns its sha256 hash
pub async fn download_with_progress_and_sha256(
    url: Url,
    to: &Path,
    on_progress: impl Fn(Progress) + Unpin,
) -> Result<String, Error> {
    let mut reader = Sha256Wrapper::new(ProgressRead::new(fetch(url).await?, on_progress));

    write_to_file(&mut reader, to).await?;

    Ok(reader.finalize())
}

async fn write_to_file<T: AsyncRead + Unpin>(reader: &mut T, to: &Path) -> Result<(), Error> {
    let partial_path = PathBuf::from(format!("{}.part", to.display()));

    let mut out = File::create(&partial_path).await?;

    let result = async {
        tokio::io::copy(reader, &mut out).await?;
        out.flush().await?;
        fs::rename(&partial_path, to).await
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(&partial_path).await;
    }

    result.map_err(Error::from)
}

/// Fetch a resource at the provided [`Url`] and return an async reader over its bytes
async fn fetch(url: Url) -> Result<Box<dyn AsyncRead + Unpin>, Error> {
    if let Some(path) = &url.to_file_path().ok() {
        Ok(Box::new(BufReader::with_capacity(
            environment::FILE_READ_BUFFER_SIZE,
            File::open(path).await?,
        )))
    } else {
        Ok(Box::new(http_get(url).await?))
    }
}

/// Internal fetch helper (sanity control) for `get`
async fn http_get(url: Url) -> Result<impl AsyncRead + Unpin, Error> {
    let response = get_client().get(url).send().await?.error_for_status()?;

    let stream = response.bytes_stream().map_err(io::Error::other);
    // Convert the stream into an AsyncReader. This chunks the stream
    // automatically and we also get compatibility with tokio::io functions.
    Ok(tokio_util::io::StreamReader::new(stream))
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("fetch")]
    Fetch(#[from] reqwest::Error),
    #[error("io")]
    Read(#[from] io::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub delta: u64,
    pub completed: u64,
}

struct ProgressRead<R, F>
where
    R: AsyncRead + Unpin,
    F: Fn(Progress) + Unpin,
{
    total: u64,
    reader: R,
    callback: F,
}

impl<R, F> ProgressRead<R, F>
where
    R: AsyncRead + Unpin,
    F: Fn(Progress) + Unpin,
{
    pub fn new(reader: R, callback: F) -> Self {
        Self {
            total: 0,
            reader,
            callback,
        }
    }
}

impl<R, F> AsyncRead for ProgressRead<R, F>
where
    R: AsyncRead + Unpin,
    F: Fn(Progress) + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> task::Poll<io::Result<()>> {
        let previous = buf.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(cx, buf);
        let delta = (buf.filled().len() - previous) as u64;
        self.total += delta;
        (self.callback)(Progress {
            completed: self.total,
            delta,
        });
        result
    }
}
