// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use fs_err as fs;
use std::{collections::HashMap, error::Error};

use moss::repository;
use tracing::error;
use tui::Styled;

mod cli;

/// Main entry point
fn main() {
    if let Err(error) = cli::process() {
        if let Some(error) = error_needs_manual_handling(&error) {
            match error {
                ManuallyHandledError::UnsupportedRepos(_) => todo!("handle unsupported repo format"),
                ManuallyHandledError::OutdatedRepos(config_manager, outdated_repos) => {
                    handle_outdated_repos(config_manager, outdated_repos);
                }
            }
        } else {
            report_error(error);
        }

        std::process::exit(1);
    }
}

/// Report an execution error to the user
fn report_error(error: cli::Error) {
    let sources = sources(&error);
    let error = sources.join(": ");
    error!(error, "Command execution failed");
    println!("{}: {error}", "Error".red());
}

/// Accumulate sources through error chains
fn sources(error: &cli::Error) -> Vec<String> {
    let mut sources = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source.take() {
        sources.push(error.to_string());
        source = error.source();
    }
    sources
}

/// Finds the error source `E` in the given errors nested sources
fn find_source<E: Error + 'static>(error: &dyn Error) -> Option<&E> {
    if let Some(source) = error.source() {
        if let Some(found) = source.downcast_ref::<E>() {
            return Some(found);
        }

        return find_source(source);
    }

    None
}

fn error_needs_manual_handling(error: &cli::Error) -> Option<ManuallyHandledError> {
    if let Some(repository::manager::Error::UnsupportedRepos(repos)) = find_source::<repository::manager::Error>(&error)
    {
        return Some(ManuallyHandledError::UnsupportedRepos(repos.clone()));
    } else if let Some(repository::manager::Error::OutdatedRepos(config_manager, repos)) =
        find_source::<repository::manager::Error>(&error)
    {
        return Some(ManuallyHandledError::OutdatedRepos(
            config_manager.clone(),
            repos.clone(),
        ));
    }
    None
}

pub enum ManuallyHandledError {
    UnsupportedRepos(Vec<repository::manager::UnsupportedRepoFormat>),
    OutdatedRepos(Option<config::Manager>, Vec<repository::manager::OutdatedRepoIndexUri>),
}

fn handle_outdated_repos(
    config_manager: Option<config::Manager>,
    outdated_repos: Vec<repository::manager::OutdatedRepoIndexUri>,
) {
    let count = outdated_repos.len();

    let repo_plural = if count == 1 { "repo" } else { "repos" };
    let require_plural = if count == 1 { "requires" } else { "require" };

    if let Some(config_manager) = &config_manager {
        println!("{count} {repo_plural} will be upgraded to the new repository format");

        let loaded_config = config_manager
            .load::<repository::Map>()
            .into_iter()
            .map(|item| {
                let no_ext = item.path.with_extension("");
                (no_ext, item)
            })
            .collect::<HashMap<_, _>>();

        let updated_config = outdated_repos
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
                let new_repo = moss::Repository {
                    source: repository::Source::RootIndex(repo.compatible_root_index_source),
                    ..old_repo.clone()
                };

                acc.entry(no_ext.clone()).or_default().add(id, new_repo);
                acc
            });

        // TODO: Prompt confirmation

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
    } else {
        // System model

        println!(
            "{count} system model {repo_plural} {require_plural} a configuration update to the new repository format"
        );

        for repo in outdated_repos {
            let id = &repo.repository.id;

            let old_repo = &repo.repository.repository;
            let new_repo = moss::Repository {
                source: repository::Source::RootIndex(repo.compatible_root_index_source),
                ..old_repo.clone()
            };

            let mut old_kdl = kdl::se::to_document(&old_repo).unwrap_or_default();
            let mut new_kdl = kdl::se::to_document(&new_repo).unwrap_or_default();

            old_kdl.autoformat();
            new_kdl.autoformat();

            println!("\nUpdate for {}", id.to_string().bold());
            println!("\n```diff");
            print_diff(&old_kdl.to_string(), &new_kdl.to_string(), None);
            println!("```");
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
