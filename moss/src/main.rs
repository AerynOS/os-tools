// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::error::Error;

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
                ManuallyHandledError::OutdatedRepos(outdated_repos) => {
                    print_outdated_repos_help(outdated_repos);
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
    } else if let Some(repository::manager::Error::OutdatedRepos(repos)) =
        find_source::<repository::manager::Error>(&error)
    {
        return Some(ManuallyHandledError::OutdatedRepos(repos.clone()));
    }
    None
}

pub enum ManuallyHandledError {
    UnsupportedRepos(Vec<repository::manager::UnsupportedRepoFormat>),
    OutdatedRepos(Vec<repository::manager::OutdatedRepoIndexUri>),
}

fn print_outdated_repos_help(outdated_repos: Vec<repository::manager::OutdatedRepoIndexUri>) {
    let count = outdated_repos.len();

    let repo_plural = if count == 1 { "repo" } else { "repos" };
    let require_plural = if count == 1 { "requires" } else { "require" };

    println!("{count} {repo_plural} {require_plural} a configuration update to the new repository format");

    for repo in outdated_repos {
        let id = &repo.repository.id;
        let config_path = &repo.repository.config_path;

        let old_repo = &repo.repository.repository;
        let new_repo = moss::Repository {
            source: repository::Source::RootIndex(repo.compatible_root_index_source),
            ..old_repo.clone()
        };

        let old_yaml = serde_yaml::to_string(&old_repo).unwrap_or_default();
        let new_yaml = serde_yaml::to_string(&new_repo).unwrap_or_default();

        if let Some(config_path) = config_path {
            println!(
                "\nRepo {} at {}",
                id.to_string().bold(),
                config_path.display().to_string().bold()
            );
        } else {
            println!("\nRepo {}", id.to_string().bold());
        }

        println!("\n```diff");
        print_diff(&old_yaml, &new_yaml);
        println!("```");
    }
}

fn print_diff(a: &str, b: &str) {
    let diff = similar::TextDiff::from_lines(a, b);

    for line in diff.unified_diff().to_string().lines() {
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
