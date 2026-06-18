// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0
use std::{error::Error, sync::Arc};

use moss::repository;
use tui::Styled;

pub use self::architecture::Architecture;
pub use self::env::Env;
pub use self::macros::Macros;
pub use self::paths::Paths;
pub use self::profile::Profile;
pub use self::recipe::Recipe;
pub use self::timing::Timing;

mod architecture;
mod build;
mod cli;
mod container;
mod draft;
mod env;
mod macros;
mod package;
mod paths;
mod profile;
mod recipe;
mod timing;
mod upstream;

fn main() {
    if let Err(error) = cli::process() {
        if let Some(error) = error_needs_manual_handling(&error) {
            match error {
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

fn report_error(error: cli::Error) {
    let sources = sources(&error);
    let error = sources.join(": ");
    eprintln!("{}: {error}", "Error".red());
}

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
    if let Some(repository::manager::Error::OutdatedRepos(config_manager, repos)) =
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
    OutdatedRepos(Arc<repository::manager::Source>, Vec<repository::OutdatedRepoIndexUri>),
}
