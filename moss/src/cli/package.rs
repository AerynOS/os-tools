// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::PathBuf};

use clap::Parser;
use info::info;
use inspect::inspect;
use list::list;
use moss::{Client, Installation, client, dependency::Provider, environment};
use stone::StoneReadError;
use thiserror::Error;
use tracing::instrument;

mod info;
mod inspect;
mod list;

use crate::cli::{Confirmation, Global};

#[derive(Debug, Parser)]
#[command(
    name = "package",
    about = "Manage installed packages and get info on available packages"
)]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

impl Command {
    /// Handles the "package" subcommand.
    pub fn handle(self, global: Global, installation: Installation) -> Result<(), Error> {
        match self.subcommand {
            Subcommand::Fetch { output_dir, packages } => fetch(global, output_dir, packages, installation),
            Subcommand::Add(args) => add(global, args, installation),
            Subcommand::Info(args) => info(args, installation),
            Subcommand::Inspect(args) => inspect(args),
            Subcommand::List(args) => list(args, installation),
            Subcommand::Extract { files, output_dir } => extract(files, output_dir),
            Subcommand::Remove(args) => remove(global, args, installation),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("No such package {0}")]
    NotFound(String),
    #[error("No packages found")]
    NoneFound,
    #[error("client")]
    Client(#[from] client::Error),
    #[error("stone format")]
    Format(#[from] StoneReadError),
    #[error("One or more files failed the integrity check")]
    ValidationFailed,
    #[error(transparent)]
    Extract(#[from] client::extract::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    #[command(visible_alias("pfe"), about = "Fetch package stone(s) by name or provider")]
    Fetch {
        /// directory to write the fetched stone(s)
        #[arg(short, long, default_value = ".")]
        output_dir: PathBuf,

        /// packages to fetch
        #[arg(name = "PACKAGE", required = true)]
        packages: Vec<Provider>,
    },

    #[command(
        visible_alias("pad"),
        about = "Add package(s) to the implicit system-model from a file path or by name in the repo"
    )]
    Add(AddArgs),

    #[command(visible_alias("pif"), about = "Show information about the package provider")]
    Info(InfoArgs),

    #[command(
        visible_alias = "pin",
        about = "Show detailed (debug) information on a local `.stone` file"
    )]
    Inspect(InspectArgs),

    #[command(
        visible_alias("pls"),
        about = "List all installed packages according to the resolution of the system-model"
    )]
    List(ListArgs),

    #[command(visible_alias = "pex", about = "Extract contents of Stone archive(s) to disk")]
    Extract {
        #[arg(help = "valid moss-format archives(s) to extract")]
        files: Vec<PathBuf>,

        #[arg(
            long = "output-dir",
            help = "directory to which to extract moss-format archive(s) (default: `.`)",
            default_value = "."
        )]
        output_dir: PathBuf,
    },

    #[command(visible_alias("prm"), about = "Removes package(s) from the implicit system-model")]
    Remove(RemoveArgs),
}

#[derive(Clone, Debug, clap::Args)]
struct AddArgs {
    #[arg(help = "Package names or providers to install")]
    providers: Vec<Provider>,
    #[arg(long)]
    reinstall: bool,
    #[arg(long, help = "Simulate the operation")]
    dry_run: bool,
    /// Blit this sync to the provided directory instead of the root.
    ///
    /// This operation won't be captured as a new state.
    #[arg(value_name = "dir", long = "to")]
    blit_target: Option<PathBuf>,
}

#[derive(Clone, Debug, clap::Args)]
struct InfoArgs {
    provider: Provider,
    #[arg(short, long, help = "Filter to list of known repositories")]
    repositories: Vec<String>,
    #[arg(short = 'f', long = "show-files", help = "Show files provided by package")]
    show_files: bool,
}

#[derive(Clone, Debug, clap::Args)]
struct InspectArgs {
    #[arg(help = "Files to inspect")]
    paths: Vec<PathBuf>,

    #[arg(short, long, help = "Check the integrity of the stone file(s)")]
    check: bool,

    #[arg(
        short,
        long,
        requires = "check",
        help = "Suppress output, only exit status indicates success or failure (requires --check)"
    )]
    quiet: bool,
}

#[derive(Clone, Debug, clap::Args)]
struct ListArgs {
    #[arg(
        short,
        long,
        help = "Filter to list of explicitly added packages in the implicit system-model"
    )]
    explicit: bool,
    #[arg(
        short,
        long,
        help = "Filter to list all available packages given the active repositories in the implicit system-model"
    )]
    available: bool,
    #[arg(short, long, help = "Filter to list of known repositories")]
    repositories: Vec<String>,
    #[arg(
        short,
        long,
        help = "Filter to list the packages that would be sync'd based on the current cached repository state"
    )]
    sync: bool,
}

#[derive(Clone, Debug, clap::Args)]
struct RemoveArgs {
    #[arg(help = "Package names or providers to remove")]
    providers: Vec<Provider>,
    #[arg(long, help = "Simulate the operation")]
    dry_run: bool,
}

/// Handle execution of `moss fetch`
#[instrument(skip_all)]
fn fetch(
    global: Global,
    output_dir: PathBuf,
    providers: Vec<Provider>,
    installation: Installation,
) -> Result<(), Error> {
    let pkgs = providers.iter().map(|prov| prov.to_string()).collect::<Vec<_>>();

    let mut client = Client::new(environment::NAME, installation)?;

    // FIXME: Maybe we want client.fetch to accept Providers?
    // It's already using them internally.
    let pkgs_str = pkgs.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    client.fetch(&pkgs_str, &output_dir, global.verbose)?;

    Ok(())
}

/// Handle execution of `moss install`
#[instrument(skip_all)]
fn add(global: Global, args: AddArgs, installation: Installation) -> Result<(), Error> {
    let pkgs = args.providers.iter().map(|prov| prov.to_string()).collect::<Vec<_>>();

    // Grab a client for the root
    let mut client = Client::new(environment::NAME, installation)?;

    // Make ephemeral if a blit target was provided
    if let Some(blit_target) = args.blit_target {
        client = client.ephemeral(blit_target)?;
    }

    // FIXME: Maybe we want client.install to accept Providers?
    // It's already using them internally.
    let pkgs_str = pkgs.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    client.install(&pkgs_str, global.confirm == Confirmation::DoNotAsk, args.dry_run)?;

    Ok(())
}

/// Handle execution of `moss remove`
#[instrument(skip_all)]
fn remove(global: Global, args: RemoveArgs, installation: Installation) -> Result<(), Error> {
    let pkgs = args.providers.iter().map(|p| p.to_string()).collect::<Vec<_>>();

    let mut client = Client::new(environment::NAME, installation)?;

    // FIXME: Maybe we want client.remove to accept Providers?
    // It's already using them internally.
    let pkgs_str = pkgs.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    client.remove(&pkgs_str, global.confirm == Confirmation::DoNotAsk, args.dry_run)?;

    Ok(())
}

fn extract(files: Vec<PathBuf>, output_dir: PathBuf) -> Result<(), Error> {
    let paths_ref = files.iter().collect();
    client::extract(paths_ref, &output_dir)?;
    Ok(())
}
