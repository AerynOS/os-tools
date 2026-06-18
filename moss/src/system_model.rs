// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::{Path, PathBuf};
use std::{collections::BTreeSet, io};

use fs_err as fs;
use thiserror::Error;

use crate::{Package, dependency, repository};

use self::decode::decode;
use self::encode::encode;

mod decode;
mod encode;
mod update;

#[derive(Debug, Clone)]
pub struct SystemModel {
    pub disable_warning: bool,
    pub repositories: repository::Map,
    pub packages: BTreeSet<dependency::Provider>,
    encoded: String,
}

impl SystemModel {
    pub fn encoded(&self) -> &str {
        &self.encoded
    }
}

#[derive(Debug, Clone)]
pub struct LoadedSystemModel {
    pub disable_warning: bool,
    pub repositories: repository::Map,
    pub packages: BTreeSet<dependency::Provider>,
    encoded: String,
    path: PathBuf,
}

impl LoadedSystemModel {
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl From<LoadedSystemModel> for SystemModel {
    fn from(system_model: LoadedSystemModel) -> Self {
        SystemModel {
            disable_warning: system_model.disable_warning,
            repositories: system_model.repositories,
            packages: system_model.packages,
            encoded: system_model.encoded,
        }
    }
}

/// Loads a [`SystemModel`] from the provided path
pub fn load(path: &Path) -> Result<Option<LoadedSystemModel>, LoadError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(LoadError::ReadFile)?;

    let SystemModel {
        disable_warning,
        repositories,
        packages,
        encoded,
    } = decode(&content)?;

    Ok(Some(LoadedSystemModel {
        disable_warning,
        repositories,
        packages,
        encoded,
        path: path.to_owned(),
    }))
}

/// Creates a new [`SystemModel`] with the given items
pub fn create(repositories: repository::Map, packages: BTreeSet<dependency::Provider>) -> SystemModel {
    let encoded = encode(&repositories, &packages);

    SystemModel {
        disable_warning: false,
        repositories,
        packages,
        encoded,
    }
}

impl SystemModel {
    /// Sync the provided packages to the [`SystemModel`].
    ///
    /// This function will retain formatting from the original system model
    /// and either delete existing packages where those do not exist in the
    /// incoming set, or append packages to the very end if those aren't
    /// already present in the system model
    pub fn sync_packages(self, packages: &[Package]) -> Result<SystemModel, UpdateError> {
        // Packages not provided by the incoming set of packages
        let packages_to_remove = self
            .packages
            .iter()
            .filter(|provider| !packages.iter().any(|package| package.meta.providers.contains(provider)))
            .collect();

        // Packages which aren't already provided by the system-model
        let packages_to_add = packages
            .iter()
            .filter(|package| {
                !package
                    .meta
                    .providers
                    .iter()
                    .any(|provider| self.packages.contains(provider))
            })
            // We add these as their package name
            .map(|package| package.meta.name.as_str());

        // Apply diffs to encoded system model which allows us to retain existing formatting
        let updated_content = update::sync_packages(&self.encoded, &packages_to_remove, packages_to_add)?;

        // Convert back into decoded system model
        Ok(decode(&updated_content)?)
    }

    /// Update the provided repositories to the [`SystemModel`].
    ///
    /// This function will retain formatting from the original system model
    /// and update existing repositories from the incoming set where
    /// they match by [`repository::Id`]
    pub fn update_repositories(self, repos: &repository::Map) -> Result<SystemModel, UpdateError> {
        let updated_content = update::update_repositories(&self.encoded, repos.iter())?;

        // Convert back into decoded system model
        Ok(decode(&updated_content)?)
    }
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("read file")]
    ReadFile(#[source] io::Error),
    #[error("decode")]
    Decode(#[from] decode::Error),
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("decode")]
    Decode(#[from] decode::Error),
}
