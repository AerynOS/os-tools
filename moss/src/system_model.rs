use std::{collections::BTreeSet, io};

use fs_err as fs;
use thiserror::Error;

use crate::{Installation, Package, dependency, repository};

use self::decode::decode;
use self::encode::encode;
use self::update::update;

mod decode;
mod encode;
mod update;

#[derive(Debug, Clone)]
pub struct SystemModel {
    pub repositories: repository::Map,
    pub packages: BTreeSet<dependency::Provider>,
    encoded: String,
}

/// Loads a [`SystemModel`] from the provided [`Installation`]
pub fn load(installation: &Installation) -> Result<Option<SystemModel>, LoadError> {
    let path = installation.system_model_path();

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).map_err(LoadError::ReadFile)?;

    Ok(Some(decode(&content)?))
}

/// Creates a new [`SystemModel`] with the given items
pub fn create(repositories: repository::Map, packages: BTreeSet<dependency::Provider>) -> SystemModel {
    let encoded = encode(&repositories, &packages);

    SystemModel {
        repositories,
        packages,
        encoded,
    }
}

impl SystemModel {
    /// Updates the [`SystemModel`] with the provided packages.
    ///
    /// This function will retain formatting from the original system model
    /// and either delete existing packages where those do not exist in the
    /// incoming set, or append packages to the very end if those aren't
    /// already present in the system model
    pub fn update(self, packages: &[Package]) -> Result<SystemModel, UpdateError> {
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
            .map(|package| package.meta.name.as_ref().as_str());

        // Apply diffs to encoded system model which allows us to retain existing formatting
        let updated_content = update(&self.encoded, &packages_to_remove, packages_to_add)?;

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
