// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use bytes::Bytes;
use fs_err::tokio::{self as fs, File};
use futures_util::{
    Stream, StreamExt,
    stream::{self, BoxStream},
};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use url::Url;

use crate::environment;

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

/// Fetch a resource at the provided [`Url`] and stream response body as bytes
pub async fn stream(url: Url) -> Result<BoxStream<'static, Result<Bytes, Error>>, Error> {
    match url_file(&url) {
        Some(path) => read(path).await,
        _ => Ok(fetch(url).await?.boxed()),
    }
}

/// Downloads a file to the provided path
pub async fn download(url: Url, to: &Path) -> Result<(), Error> {
    download_with_progress(url, to, |_| {}).await
}

/// Downloads a file to the provided path and invokes `on_progress` after each
/// chunk is downloaded
pub async fn download_with_progress(url: Url, to: &Path, on_progress: impl Fn(Progress)) -> Result<(), Error> {
    let partial_path = PathBuf::from(format!("{}.part", to.display()));

    let mut bytes = stream(url).await?;
    let mut out = File::create(&partial_path).await?;

    let mut total = 0;

    while let Some(chunk) = bytes.next().await {
        let bytes = chunk?;
        let delta = bytes.len() as u64;
        total += delta;
        out.write_all(&bytes).await?;

        (on_progress)(Progress {
            delta,
            completed: total,
        });
    }

    out.flush().await?;

    fs::rename(partial_path, to).await?;

    Ok(())
}

/// Internal fetch helper (sanity control) for `get`
async fn fetch(url: Url) -> Result<impl Stream<Item = Result<Bytes, Error>>, Error> {
    let response = get_client().get(url).send().await?;

    response
        .error_for_status()
        .map(reqwest::Response::bytes_stream)
        .map(|stream| stream.map(|result| result.map_err(Error::Fetch)))
        .map_err(Error::Fetch)
}

/// Asynchronously read a filesystem path akin to the fetch API
async fn read(path: PathBuf) -> Result<BoxStream<'static, Result<Bytes, Error>>, Error> {
    let mut file = File::open(path).await?;
    let size = file.metadata().await?.len() as usize;

    if size > environment::FILE_READ_CHUNK_THRESHOLD {
        let stream = ReaderStream::with_capacity(file, environment::FILE_READ_BUFFER_SIZE);

        Ok(stream.map(|result| result.map_err(Error::Read)).boxed())
    } else {
        let mut bytes = Vec::with_capacity(size);
        file.read_to_end(&mut bytes).await?;

        Ok(stream::once(async move { Ok(bytes.into()) }).boxed())
    }
}

/// Specialise handling of `file://` URLs for fetching
fn url_file(url: &Url) -> Option<PathBuf> {
    if url.scheme() == "file" {
        url.to_file_path().ok()
    } else {
        None
    }
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
