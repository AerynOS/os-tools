// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::HashMap;

use fs_err as fs;
use tui::Styled;
use url::Url;

use crate::{
    Repository, SystemModel,
    repository::{self, manager},
};

#[derive(Debug, Clone)]
pub struct OutdatedRepoIndexUri {
    pub repository: repository::Cached,
    pub legacy_index_uri: Url,
    pub compatible_root_index_source: repository::RootIndexSource,
}

pub fn handle_outdated_index_uris(source: &manager::Source, outdated_repos: Vec<OutdatedRepoIndexUri>) {
    let count = outdated_repos.len();

    let repo_plural = if count == 1 { "repo" } else { "repos" };
    let require_plural = if count == 1 { "requires" } else { "require" };

    match source {
        manager::Source::ConfigManager(config_manager) => {
            println!("{count} {repo_plural} will be upgraded to the new repository format");

            let loaded_config = config_manager
                .load::<repository::Map>()
                .into_iter()
                .map(|item| {
                    let no_ext = item.path.with_extension("");
                    (no_ext, item)
                })
                .collect::<HashMap<_, _>>();

            let updated_config =
                outdated_repos
                    .into_iter()
                    .fold(HashMap::<_, repository::Map>::new(), |mut acc, repo| {
                        let Some(path) = repo.repository.config_path else {
                            // Unreachable, if config manager is present than
                            // all repos were loaded from it & will have a config path
                            return acc;
                        };
                        let no_ext = path.with_extension("");

                        let id = repo.repository.id.clone();

                        let old_repo = &repo.repository.repository;
                        let new_repo = Repository {
                            source: repository::Source::RootIndex(repo.compatible_root_index_source),
                            ..old_repo.clone()
                        };

                        acc.entry(no_ext.clone()).or_default().add(id, new_repo);
                        acc
                    });

            for (no_ext, updated_map) in updated_config {
                let Some(current_config) = loaded_config.get(&no_ext) else {
                    // Unreachable, everything returned originated from
                    // stuff loaded via config manager
                    continue;
                };

                let name = no_ext.file_name().unwrap_or_default().to_str().unwrap_or_default();
                let kdl_path = no_ext.with_extension("kdl");

                let old_content = fs::read_to_string(&current_config.path).unwrap_or_default();

                if let Err(e) = config_manager.save(name, &updated_map) {
                    eprintln!("Failed to save updated config to {kdl_path:?}: {e:#}");
                    continue;
                }

                let new_content = fs::read_to_string(&kdl_path).unwrap_or_default();

                println!("\nUpdate applied to {kdl_path:?}");

                println!("\n```diff");
                print_diff(
                    &old_content,
                    &new_content,
                    Some((
                        current_config.path.as_os_str().to_str().unwrap_or_default(),
                        kdl_path.as_os_str().to_str().unwrap_or_default(),
                    )),
                );
                println!("```");
            }
        }
        manager::Source::SystemModel { system_model, .. } => {
            println!(
                "{count} system-model {repo_plural} {require_plural} a configuration update to the new repository format"
            );

            let path = system_model.path().to_owned();

            let updated_repos = outdated_repos
                .iter()
                .map(|repo| {
                    (
                        repo.repository.id.clone(),
                        Repository {
                            source: repository::Source::RootIndex(repo.compatible_root_index_source.clone()),
                            ..repo.repository.repository.clone()
                        },
                    )
                })
                .collect::<repository::Map>();

            let updated_system_model = SystemModel::from(system_model.clone())
                .update_repositories(&updated_repos)
                .expect("roundtrip system model update");

            let old_kdl = system_model.encoded();
            let new_kdl = updated_system_model.encoded();

            println!("\nApply the following update to {path:?}");

            println!("\n```diff");
            print_diff(
                old_kdl,
                new_kdl,
                Some((
                    path.as_os_str().to_str().unwrap_or_default(),
                    path.as_os_str().to_str().unwrap_or_default(),
                )),
            );
            println!("```");
        }
        manager::Source::Explicit { .. } => {
            println!("{count} {repo_plural} {require_plural} a configuration update to the new repository format");

            for repo in outdated_repos {
                let id = &repo.repository.id;

                let old_repo = &repo.repository.repository;
                let new_repo = Repository {
                    source: repository::Source::RootIndex(repo.compatible_root_index_source),
                    ..old_repo.clone()
                };

                let mut old_kdl = kdl::se::to_document(&old_repo).unwrap_or_default();
                let mut new_kdl = kdl::se::to_document(&new_repo).unwrap_or_default();

                old_kdl.autoformat();
                new_kdl.autoformat();

                println!("\nUpdate for repository {}", id.to_string().bold());
                println!("\n```diff");
                print_diff(&old_kdl.to_string(), &new_kdl.to_string(), None);
                println!("```");
            }
        }
    }
}

fn print_diff(a: &str, b: &str, header: Option<(&str, &str)>) {
    let diff = similar::TextDiff::from_lines(a, b);

    let mut unified = diff.unified_diff();

    if let Some((file_a, file_b)) = header {
        unified.header(file_a, file_b);
    }

    for line in unified.to_string().lines() {
        let colored = if line.starts_with('-') {
            line.red()
        } else if line.starts_with('+') {
            line.green()
        } else {
            line.dim()
        };

        println!("{colored}");
    }
}
