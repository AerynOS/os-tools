// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use derive_more::{Debug, Display, From, Into};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io;
use url::Url;

use config::Config;

use crate::{db::meta, request};

pub use self::format::Format;
pub use self::manager::Manager;

pub mod format;
pub mod manager;

pub const DEFAULT_CHANNEL: &str = "main";
pub const DEFAULT_ARCH: &str = "x86_64";

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
    #[serde(flatten)]
    pub source: Source,
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
    index_uri: Arc<ArcSwap<Option<Url>>>,
}

impl Cached {
    pub fn new(id: Id, repository: Repository, db: meta::Database, index_uri: Option<Url>) -> Self {
        Self {
            id,
            repository,
            db,
            index_uri: Arc::new(ArcSwap::new(Arc::new(index_uri))),
        }
    }

    /// Resolved index uri from a repository [`Source`]
    ///
    /// Is `None` if the [`Source`] has not yet been resolved,
    /// in the case of a `root-index` source
    pub fn index_uri(&self) -> Option<Url> {
        self.index_uri.load().as_ref().clone()
    }

    fn set_index_uri(&self, uri: Url) {
        self.index_uri.swap(Arc::new(Some(uri)));
    }
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

impl<'a> IntoIterator for &'a Map {
    type Item = (&'a Id, &'a Repository);
    type IntoIter = std::collections::btree_map::Iter<'a, Id, Repository>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
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
    request::download(url, &out_path.into()).await?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("request")]
    Request(#[from] request::Error),
    #[error("io")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "uri")]
    DirectIndex(Url),
    #[serde(untagged)]
    RootIndex(RootIndexSource),
}

impl Source {
    pub fn direct_index(&self) -> Option<&Url> {
        if let Self::DirectIndex(url) = self {
            Some(url)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootIndexSource {
    pub base_uri: Url,
    #[serde(default = "default_channel")]
    pub channel: format::Identifier,
    pub version: format::ScopedIdentifier,
    #[serde(default = "default_arch")]
    pub arch: String,
}

impl RootIndexSource {
    pub fn uri(&self) -> Url {
        let mut uri = self.base_uri.clone();
        let mut path = uri.path().to_owned();

        if !path.ends_with('/') {
            path.push('/');
        }

        path.push_str(self.channel.as_ref());
        path.push('/');
        path.push_str("moss-root.json");

        uri.set_path(&path);

        uri
    }

    pub fn history_index_uri(&self, ident: &format::Identifier) -> Url {
        let mut uri = self.base_uri.clone();
        let mut path = uri.path().to_owned();

        if !path.ends_with('/') {
            path.push('/');
        }

        path.push_str(self.channel.as_ref());
        path.push_str("/history/");
        path.push_str(ident.as_ref());
        path.push('/');
        path.push_str(&self.arch);
        path.push_str("/stone.index");

        uri.set_path(&path);

        uri
    }
}

fn default_channel() -> format::Identifier {
    DEFAULT_CHANNEL.try_into().expect("valid identifier")
}

fn default_arch() -> String {
    DEFAULT_ARCH.to_owned()
}
