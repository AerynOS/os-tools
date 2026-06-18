// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, slice};

use thiserror::Error;
use tracing::{Instrument, instrument};

use crate::{
    Installation, Provider, Repository,
    client::{self, Client},
    db, dependency, package, repository, runtime,
    state::{self, Selection},
};

// TODO: Flesh this out before v0 -> v1 upgrade is in place
#[allow(unused, clippy::diverging_sub_expression)]
#[instrument(skip(client))]
pub fn self_upgrade(client: &mut Client, simulate: bool) -> Result<(), Error> {
    // Ensure client is stateful
    if client.is_ephemeral() {
        return todo!("error can't self upgrade with epehemeral client");
    }

    // Get previously installed moss
    let installed = client.registry.list_installed().collect::<Vec<_>>();
    let Some(prev_moss) = installed.into_iter().find(|p| p.meta.name.as_str() == "moss") else {
        return todo!("error can't self upgrade without moss installed in current state");
    };

    // Get the list of repos that are unsupported by this version of moss
    let Some(unsupported_repos) = runtime::block_on(
        async {
            match client.refresh_repositories().await {
                Err(client::Error::Repository(repository::manager::Error::UnsupportedRepos(repos))) => Ok(Some(repos)),
                Err(err) => Err(Error::Client(err)),
                Ok(_) => Ok(None),
            }
        }
        .in_current_span(),
    )?
    else {
        println!("moss already supports latest format for all repositories");
        return Ok(());
    };

    // Tempdir to fetch intermediate `upgrade_via` index files
    // to so we can find the `moss` package from this index
    let temp_dir = tempfile::tempdir().expect("TODO");

    // If multiple repos are unsupported & have moss, upgrade
    // from the highest priority repo
    let mut moss_priority_map = BTreeMap::new();
    let mut missing_upgrade_via = vec![];
    let mut missing_moss = vec![];

    for unsupported_repo in unsupported_repos {
        match &unsupported_repo.upgrade_via_index_uri {
            Some(uri) => {
                let mut temp_client = Client::builder(
                    "temp-self-upgrade",
                    Installation::open(temp_dir.path(), None).expect("TODO"),
                )
                .repositories(repository::Map::from_iter([(
                    unsupported_repo.repository.id.clone(),
                    Repository {
                        description: "...".to_owned(),
                        source: repository::Source::DirectIndex(uri.clone()),
                        priority: 0.into(),
                        active: true,
                    },
                )]))
                .build()?;

                runtime::block_on(temp_client.ensure_repos_initialized().in_current_span())?;

                let packages = temp_client.lookup_packages_by_provider(
                    &Provider {
                        kind: dependency::Kind::PackageName,
                        name: "moss".to_owned(),
                    },
                    package::Flags::new().with_available(),
                );

                if let Some(package) = packages.first() {
                    moss_priority_map.insert(unsupported_repo.repository.repository.priority, package.clone());
                } else {
                    missing_moss.push(unsupported_repo.repository.id.clone());
                }
            }
            None => {
                missing_upgrade_via.push(unsupported_repo.repository.id.clone());
            }
        }
    }

    let Some(new_moss) = moss_priority_map.values().next_back() else {
        if !missing_moss.is_empty() {
            for repo in missing_moss {
                eprintln!("`moss` doesn't exist in repository {repo}");
            }
        }
        if !missing_upgrade_via.is_empty() {
            for repo in missing_upgrade_via {
                eprintln!("Repository {repo} is missing an `upgrade_via` format attribute");
            }
        }

        return todo!("error for no unsupported repo w/ upgrade_via moss available");
    };

    // Cache new moss
    runtime::block_on(client.cache_packages(slice::from_ref(new_moss)).in_current_span())?;

    // Calculate the new state of packages (prev_state - prev_moss + new_moss)
    let new_state_pkgs = {
        let mut previous_selections = match client.installation.active_state {
            Some(id) => {
                client
                    .state_db
                    .get(id)
                    .map_err(|err| Error::MissingActiveStateFromDb(err, id))?
                    .selections
            }
            _ => vec![],
        };

        let moss_selection = if let Some(idx) = previous_selections
            .iter()
            .position(|selection| selection.package == prev_moss.id)
        {
            // Remove old moss selection
            let old_selection = previous_selections.remove(idx);

            // Update it w/ new id
            Selection {
                package: new_moss.id.clone(),
                ..old_selection
            }
        } else {
            return todo!("internal error, installed moss not listed as state selection");
        };

        // Add it
        previous_selections
            .into_iter()
            .chain(Some(moss_selection))
            .collect::<Vec<_>>()
    };

    // Perfect, apply state.
    client.new_state(&new_state_pkgs, "Self upgrade")?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("get state {0} from state_db")]
    MissingActiveStateFromDb(#[source] db::Error, state::Id),
}
