// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{error::Error, sync::Arc};

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
                ManuallyHandledError::OutdatedRepos(manager_source, outdated_repos) => {
                    repository::handle_outdated_index_uris(&manager_source, outdated_repos);
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
    OutdatedRepos(Arc<repository::manager::Source>, Vec<repository::OutdatedRepoIndexUri>),
}
