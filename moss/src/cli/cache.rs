use clap::Parser;
use moss::{Client, Installation, client, environment};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(about = "Managed cached data")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    #[command(
        about = "Prune cached artefacts",
        long_about = "Prune cached artefacts

This will remove all downloaded stones & unpacked asset data for packages not in any state or active repository."
    )]
    Prune,
}

pub fn handle(command: Command, installation: Installation) -> Result<(), Error> {
    match command.subcommand {
        Subcommand::Prune => handle_prune(installation),
    }
}

fn handle_prune(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::SetupClient)?;

    let num_removed_files = client.prune_cache().map_err(Error::PruneCache)?;

    if num_removed_files > 0 {
        let s = if num_removed_files > 1 { "s" } else { "" };

        println!("{num_removed_files} file{s} removed");
    } else {
        println!("No files to remove");
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to setup moss client")]
    SetupClient(#[source] client::Error),
    #[error("failed to prune cache")]
    PruneCache(#[source] client::Error),
}
