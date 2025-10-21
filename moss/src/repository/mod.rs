// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::path::PathBuf;

use derive_more::{Debug, Display, From, Into};
use fs_err::tokio::File;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{self, AsyncWriteExt};
use url::Url;

use config::Config;

use crate::{db::meta, request};

pub use self::manager::Manager;

pub mod manager;

/// A unique [`Repository`] identifier
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Ord, PartialOrd, From, Display)]
#[debug("{_0:?}")]
#[serde(from = "String")]
pub struct Id(String);

impl Id {
    pub fn new(identifier: &str) -> Self {
        Self(
            identifier
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
                .collect(),
        )
    }
}

/// Repository configuration data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub description: String,
    pub uri: Url,
    pub priority: Priority,
    #[serde(default = "default_as_true")]
    pub active: bool,
}

fn default_as_true() -> bool {
    true
}

/// A repository that has been
/// fetched and cached to a meta database
#[derive(Debug, Clone)]
pub struct Cached {
    pub id: Id,
    pub repository: Repository,
    pub db: meta::Database,
}

/// The selection priority of a [`Repository`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, Into)]
pub struct Priority(u64);

impl Priority {
    pub fn new(priority: u64) -> Self {
        Self(priority)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0).reverse()
    }
}

/// A map of repositories
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Map(BTreeMap<Id, Repository>);

impl Map {
    pub fn with(items: impl IntoIterator<Item = (Id, Repository)>) -> Self {
        Self(items.into_iter().collect())
    }

    pub fn get(&self, id: &Id) -> Option<&Repository> {
        self.0.get(id)
    }

    pub fn add(&mut self, id: Id, repo: Repository) {
        self.0.insert(id, repo);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Id, &Repository)> {
        self.0.iter()
    }

    pub fn merge(self, other: Self) -> Self {
        Self(self.0.into_iter().chain(other.0).collect())
    }
}

impl IntoIterator for Map {
    type Item = (Id, Repository);
    type IntoIter = std::collections::btree_map::IntoIter<Id, Repository>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(Id, Repository)> for Map {
    fn from_iter<T: IntoIterator<Item = (Id, Repository)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Config for Map {
    fn domain() -> String {
        "repo".into()
    }
}

async fn fetch_index(url: Url, out_path: impl Into<PathBuf>) -> Result<(), FetchError> {
    let mut stream = request::get(url).await?;

    let mut out = File::create(out_path).await?;

    while let Some(chunk) = stream.next().await {
        out.write_all(&chunk?).await?;
    }

    out.flush().await?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("request")]
    Request(#[from] request::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
