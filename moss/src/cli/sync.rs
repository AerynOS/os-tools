// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::Parser;
use moss::{Installation, client::Client, environment, runtime};
use tracing::instrument;

pub use moss::client::Error;

use crate::cli::{Confirmation, Global};

#[derive(Debug, Parser)]
#[command(
    name = "sync",
    visible_aliases = ["sy", "up"],
    about = "Sync packages",
    long_about = "Sync package selections with candidates from the highest priority repository"
)]
pub struct Command {
    /// Update repositories before syncing
    #[arg(short, long)]
    update: bool,
    /// Blit this sync to the provided directory instead of the root
    ///
    /// This operation won't be captured as a new state
    #[arg(value_name = "dir", long = "to")]
    blit_target: Option<PathBuf>,

    /// Simulate the sync (dry-run)
    #[arg(long)]
    dry_run: bool,

    /// Sync against the provided system-model.kdl
    ///
    /// Only the repositories and packages from the provided file
    /// will be used to create the new state
    #[arg(value_name = "file", long)]
    import: Option<PathBuf>,
}

impl Command {
    #[instrument(skip_all)]
    pub fn handle(self, global: Global, installation: Installation) -> Result<(), Error> {
        let mut client_builder = Client::builder(environment::NAME, installation);

        if let Some(path) = self.import {
            client_builder = client_builder.system_model_path(path);
        }

        // Make ephemeral if a blit target was provided
        if let Some(blit_target) = self.blit_target {
            client_builder = client_builder.ephemeral(blit_target);
        }

        let mut client = client_builder.build()?;

        // Update repos if requested
        if self.update {
            runtime::block_on(client.refresh_repositories())?;
        }

        client.sync(global.confirm == Confirmation::DoNotAsk, self.dry_run)?;

        Ok(())
    }
}
