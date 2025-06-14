// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs_err::{self as fs, File};
use futures_util::{StreamExt, TryStreamExt, stream};
use thiserror::Error;
use xxhash_rust::xxh3::xxh3_64;

use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::db::meta;
use crate::repository::{self, Repository};
use crate::{Installation, package};
use crate::{environment, runtime};

enum Source {
    System(config::Manager),
    Explicit { identifier: String, repos: repository::Map },
}

impl Source {
    fn identifier(&self) -> &str {
        match self {
            Source::System(_) => environment::NAME,
            Source::Explicit { identifier, .. } => identifier,
        }
    }
}

/// Manage a bunch of repositories
pub struct Manager {
    source: Source,
    installation: Installation,
    repositories: BTreeMap<repository::Id, repository::Cached>,
}

impl Manager {
    pub fn is_explicit(&self) -> bool {
        matches!(self.source, Source::Explicit { .. })
    }

    /// Create a [`Manager`] for the supplied [`Installation`] using system configurations
    pub fn system(config: config::Manager, installation: Installation) -> Result<Self, Error> {
        Self::new(Source::System(config), installation)
    }

    /// Create a [`Manager`] for the supplied [`Installation`] using the provided configurations
    ///
    /// [`Manager`] can't be used to `add` new repos in this mode
    pub fn explicit(
        identifier: impl ToString,
        repos: repository::Map,
        installation: Installation,
    ) -> Result<Self, Error> {
        Self::new(
            Source::Explicit {
                identifier: identifier.to_string(),
                repos,
            },
            installation,
        )
    }

    fn new(source: Source, installation: Installation) -> Result<Self, Error> {
        let configs = match &source {
            Source::System(config) =>
            // Load all configs, default if none exist
            {
                config
                    .load::<repository::Map>()
                    .into_iter()
                    .reduce(repository::Map::merge)
                    .unwrap_or_default()
            }
            Source::Explicit { repos, .. } => repos.clone(),
        };

        // Open all repo meta dbs and collect into hash map
        let repositories = configs
            .into_iter()
            .map(|(id, repository)| {
                let db = open_meta_db(source.identifier(), &repository, &installation)?;

                Ok((id.clone(), repository::Cached { id, repository, db }))
            })
            .collect::<Result<_, Error>>()?;

        Ok(Self {
            source,
            installation,
            repositories,
        })
    }

    /// Add a [`Repository`]
    pub fn add_repository(&mut self, id: repository::Id, repository: Repository) -> Result<(), Error> {
        let Source::System(config) = &self.source else {
            return Err(Error::ExplicitUnsupported);
        };

        // Save repo as new config file
        // We save it as a map for easy merging across
        // multiple configuration files
        {
            let map = repository::Map::with([(id.clone(), repository.clone())]);
            config.save(&id, &map).map_err(Error::SaveConfig)?;
        }

        let db = open_meta_db(self.source.identifier(), &repository, &self.installation)?;

        self.repositories
            .insert(id.clone(), repository::Cached { id, repository, db });

        Ok(())
    }

    /// Refresh a [`Repository`] by Id
    pub async fn refresh(&self, id: &repository::Id) -> Result<(), Error> {
        let Some(repo) = self.repositories.get(id).cloned() else {
            return Err(Error::UnknownRepo(id.clone()));
        };

        if repo.repository.active {
            let file = fetch_index(self.source.identifier(), &repo, &self.installation).await?;
            runtime::unblock(move || update_meta_db(&repo, &file)).await?;
        }

        Ok(())
    }

    /// Refresh all [`Repository`]'s by fetching it's latest index
    /// file and updating it's associated meta database
    pub async fn refresh_all(&mut self) -> Result<(), Error> {
        let mpb = MultiProgress::new();

        // Fetch index files asynchronously and then
        // update to DB
        stream::iter(self.repositories.iter().filter(|(_, r)| r.repository.active))
            .map(|(id, _)| async {
                let pb = mpb.add(
                    ProgressBar::new_spinner()
                        .with_style(
                            ProgressStyle::with_template(" {spinner} {wide_msg}")
                                .unwrap()
                                .tick_chars("--=≡■≡=--"),
                        )
                        .with_message(format!("{} {}", "Refreshing".blue(), *id)),
                );
                pb.enable_steady_tick(Duration::from_millis(150));

                self.refresh(id).await?;

                pb.suspend(|| println!("{} {}", "Refreshed".green(), *id));

                Ok(())
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect()
            .await
    }

    /// Ensures all repositories are initialized - index file downloaded and meta db
    /// populated.
    ///
    /// This is useful to call when initializing the moss client in-case users added configs
    /// manually outside the CLI
    pub async fn ensure_all_initialized(&mut self) -> Result<usize, Error> {
        let uninitialized = self
            .repositories
            .iter()
            .filter(|(_, r)| r.repository.active)
            .filter_map(|(id, state)| {
                let index_file =
                    cache_dir(self.source.identifier(), &state.repository, &self.installation).join("stone.index");

                if !index_file.exists() { Some(id) } else { None }
            })
            .collect::<Vec<_>>();

        if uninitialized.is_empty() {
            return Ok(0);
        }

        let mpb = MultiProgress::new();

        // Fetch index files asynchronously and then
        // update to DB
        stream::iter(&uninitialized)
            .map(|id| async {
                let pb = mpb.add(
                    ProgressBar::new_spinner()
                        .with_style(
                            ProgressStyle::with_template(" {spinner} {wide_msg}")
                                .unwrap()
                                .tick_chars("--=≡■≡=--"),
                        )
                        .with_message(format!("{} {}", "Refreshing".blue(), *id)),
                );
                pb.enable_steady_tick(Duration::from_millis(150));

                self.refresh(id).await?;

                pb.suspend(|| println!("{} {}", "Refreshed".green(), *id));

                Ok(()) as Result<_, Error>
            })
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect::<()>()
            .await?;

        Ok(uninitialized.len())
    }

    /// Returns the active repositories held by this manager
    pub(crate) fn active(&self) -> impl Iterator<Item = repository::Cached> + '_ {
        self.repositories.values().filter(|c| c.repository.active).cloned()
    }

    /// Remove a repository, deleting any related config & cached data
    pub fn remove(&mut self, id: impl Into<repository::Id>) -> Result<Removal, Error> {
        // Only allow removal for system repo manager
        let Source::System(config) = &self.source else {
            return Err(Error::ExplicitUnsupported);
        };

        // Remove from memory
        let Some(repo) = self.repositories.remove(&id.into()) else {
            return Ok(Removal::NotFound);
        };

        let cache_dir = cache_dir(self.source.identifier(), &repo.repository, &self.installation);

        // Remove cache
        if cache_dir.exists() {
            fs::remove_dir_all(&cache_dir).map_err(Error::RemoveDir)?;
        }

        // Delete config, only succeeds for configs that live in their
        // own config file w/ matching repo name
        if config.delete::<repository::Map>(&repo.id).is_err() {
            return Ok(Removal::ConfigDeleted(false));
        }

        Ok(Removal::ConfigDeleted(true))
    }

    /// List all of the known repositories
    pub fn list(&self) -> impl ExactSizeIterator<Item = (&repository::Id, &Repository)> {
        self.repositories.iter().map(|(id, state)| (id, &state.repository))
    }

    /// Sets the repo as active or not
    async fn set_active(&mut self, id: &repository::Id, active: bool) -> Result<(), Error> {
        // Only allow disable for system repo manager
        let Source::System(config) = &self.source else {
            return Err(Error::ExplicitUnsupported);
        };

        let Some(cached) = self.repositories.get_mut(id) else {
            return Err(Error::UnknownRepo(id.clone()));
        };

        if active != cached.repository.active {
            cached.repository.active = active;

            let map = repository::Map::with([(id.clone(), cached.repository.clone())]);
            config.save(id, &map).map_err(Error::SaveConfig)?;
        }

        Ok(())
    }

    /// Enable the repo
    pub async fn enable(&mut self, id: &repository::Id) -> Result<(), Error> {
        self.set_active(id, true).await
    }

    /// Disable the repo
    pub async fn disable(&mut self, id: &repository::Id) -> Result<(), Error> {
        self.set_active(id, false).await
    }
}

/// Directory for the repo cached data (db & stone index), hashed by identifier & repo URI
fn cache_dir(identifier: &str, repo: &Repository, installation: &Installation) -> PathBuf {
    let hash = format!("{:02x}", xxh3_64(format!("{identifier}-{}", repo.uri).as_bytes()));
    installation.repo_path(hash)
}

/// Open the meta db file, ensuring it's
/// directory exists
fn open_meta_db(identifier: &str, repo: &Repository, installation: &Installation) -> Result<meta::Database, Error> {
    let dir = cache_dir(identifier, repo, installation);

    fs::create_dir_all(&dir).map_err(Error::CreateDir)?;

    let db = meta::Database::new(dir.join("db").to_str().unwrap_or_default())?;

    Ok(db)
}

/// Fetches a stone index file from the repository URL
/// and saves it to the repo installation path
async fn fetch_index(
    identifier: &str,
    state: &repository::Cached,
    installation: &Installation,
) -> Result<PathBuf, Error> {
    let out_dir = cache_dir(identifier, &state.repository, installation);

    tokio::fs::create_dir_all(&out_dir).await.map_err(Error::CreateDir)?;

    let out_path = out_dir.join("stone.index");

    // Fetch index & write to `out_path`
    repository::fetch_index(state.repository.uri.clone(), &out_path).await?;

    Ok(out_path)
}

/// Updates a stones metadata into the meta db
fn update_meta_db(state: &repository::Cached, index_path: &Path) -> Result<(), Error> {
    // Wipe db since we're refreshing from a new index file
    state.db.wipe()?;

    // Get a stream of payloads
    let mut file = File::open(index_path).map_err(Error::OpenIndex)?;
    let mut reader = stone::read(&mut file)?;

    let payloads = reader.payloads()?.collect::<Result<Vec<_>, _>>()?;

    // Construct Meta for each payload
    let packages = payloads
        .into_iter()
        .filter_map(|payload| {
            if let stone::read::PayloadKind::Meta(meta) = payload {
                Some(meta)
            } else {
                None
            }
        })
        .map(|payload| {
            let meta = package::Meta::from_stone_payload(&payload.body)?;

            // Create id from hash of meta
            let hash = meta
                .hash
                .clone()
                .ok_or(Error::MissingMetaField(stone::payload::meta::Tag::PackageHash))?;
            let id = package::Id::from(hash);

            Ok((id, meta))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    // Batch add to db
    state.db.batch_add(packages)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Can't modify repos when using explicit configs")]
    ExplicitUnsupported,
    #[error("Missing metadata field: {0:?}")]
    MissingMetaField(stone::payload::meta::Tag),
    #[error("create directory")]
    CreateDir(#[source] io::Error),
    #[error("remove directory")]
    RemoveDir(#[source] io::Error),
    #[error("fetch index file")]
    FetchIndex(#[from] repository::FetchError),
    #[error("open index file")]
    OpenIndex(#[source] io::Error),
    #[error("read index file")]
    ReadStone(#[from] stone::read::Error),
    #[error("meta db")]
    Database(#[from] meta::Error),
    #[error("save config")]
    SaveConfig(#[source] config::SaveError),
    #[error("unknown repo")]
    UnknownRepo(repository::Id),
}

impl From<package::MissingMetaFieldError> for Error {
    fn from(error: package::MissingMetaFieldError) -> Self {
        Self::MissingMetaField(error.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Removal {
    NotFound,
    ConfigDeleted(bool),
}
