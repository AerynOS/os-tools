// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::PathBuf};

use clap::{Args, CommandFactory, Parser};
use clap_complete::{
    generate_to,
    shells::{Bash, Fish, Zsh},
};
use clap_mangen::Man;
use fs_err::{self as fs, File};
use moss::{Installation, client, installation};
use thiserror::Error;
use tui::Styled;

mod boot;
mod cache;
mod index;
mod package;
mod repo;
mod search;
mod search_file;
mod state;
mod sync;
mod version;

/// Generate the new CLI command structure
#[derive(Debug, Parser)]
pub struct Command {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    subcommand: Option<Subcommand>,
}

impl Command {
    /// Run the CLI according to users' flags and arguments.
    pub fn run(self) -> Result<(), Error> {
        // Prints the cli's information about version at startup
        if self.global.version {
            println!("moss {}", tools_buildinfo::get_full_version());
        }

        if let Some(dir) = self.global.generate_manpages {
            fs::create_dir_all(&dir)?;
            let main_cmd = Command::command();
            // Generate man page for the main command
            let main_man = Man::new(main_cmd.clone());
            let mut buffer = File::create(dir.join("moss.1"))?;
            main_man.render(&mut buffer)?;

            // Generate man pages for all subcommands
            for sub in main_cmd.get_subcommands() {
                let sub_man = Man::new(sub.clone());
                let name = format!("moss-{}.1", sub.get_name());
                let mut buffer = File::create(dir.join(&name))?;
                sub_man.render(&mut buffer)?;

                for nested in sub.get_subcommands() {
                    let nested_man = Man::new(nested.clone());
                    let name = format!("moss-{}-{}.1", sub.get_name(), nested.get_name());
                    let mut buffer = File::create(dir.join(&name))?;
                    nested_man.render(&mut buffer)?;
                }
            }
            return Ok(());
        }

        if let Some(dir) = self.global.generate_completions {
            fs::create_dir_all(&dir)?;
            let mut cmd = Command::command();
            generate_to(Bash, &mut cmd, "moss", &dir)?;
            generate_to(Fish, &mut cmd, "moss", &dir)?;
            generate_to(Zsh, &mut cmd, "moss", &dir)?;
            return Ok(());
        }

        // Print the version, but not if the user is using the version subcommand
        if self.global.verbose {
            match self.subcommand {
                Some(Subcommand::Version(_)) => (),
                _ => version::print(),
            }
        }

        // The default is "/" in the absence of an explicit arg.
        let installation = Installation::open(
            self.global.root_dir.clone().unwrap_or_default(),
            self.global.cache_dir.clone(),
        )?;

        if let Some(system_model) = installation.system_model.as_ref() {
            if !system_model.disable_warning {
                print_system_model_warning(&installation, false);
            } else if self.global.verbose {
                print_system_model_warning(&installation, true);
            }
        }

        match self.subcommand {
            Some(Subcommand::Boot(cmd)) => cmd.handle(installation)?,
            Some(Subcommand::Cache(cmd)) => cmd.handle(installation)?,
            Some(Subcommand::Index(cmd)) => cmd.handle()?,
            Some(Subcommand::Package(cmd)) => cmd.handle(self.global, installation)?,
            Some(Subcommand::Repo(cmd)) => cmd.handle(installation)?,
            Some(Subcommand::Search(cmd)) => cmd.handle(installation)?,
            Some(Subcommand::SearchFile(cmd)) => cmd.handle(installation)?,
            Some(Subcommand::State(cmd)) => cmd.handle(self.global, installation)?,
            Some(Subcommand::Sync(cmd)) => cmd.handle(self.global, installation)?,
            Some(Subcommand::Version(cmd)) => cmd.handle(),
            None => unreachable!(),
        }

        Ok(())
    }
}

/// Globally available arguments
#[derive(Debug, Args)]
pub struct Global {
    #[arg(
        short,
        long = "verbose",
        help = "Prints additional information about what moss is doing",
        default_value = "false",
        global = true
    )]
    pub verbose: bool,
    #[arg(
        short = 'V',
        long,
        help = "Prints out version information and exits",
        default_value = "false",
        global = true
    )]
    pub version: bool,
    #[arg(
        short = 'D',
        long = "directory",
        help = "Root directory",
        default_value = "/",
        global = true
    )]
    pub root_dir: Option<PathBuf>,
    #[arg(long, help = "Cache directory", global = true)]
    pub cache_dir: Option<PathBuf>,
    #[arg(
        long,
        help = "Logging configuration: <level>[:<format>][:<destination>]\nLevels: trace, debug, info, warn, error\nFormats: text, json\nDestinations: stderr, <file>",
        global = true
    )]
    log: Option<String>,
    #[arg(short = 'y', long = "yes", action = clap::ArgAction::SetTrue, help = "Assume yes for all questions", global = true)]
    pub confirm: Confirmation,
    #[arg(
        long = "generate-manpages",
        help = "Generate man pages in specified directory",
        value_name = "DIR",
        hide = true
    )]
    generate_manpages: Option<PathBuf>,
    #[arg(
        long = "generate-completions",
        help = "Generate shell completions in specified directory",
        value_name = "DIR",
        hide = true
    )]
    generate_completions: Option<String>,
}

/// Whether to interactively ask the
/// user for confirmation or not.
/// When [Confirmation::DoNotAsk], the operation will continue
/// without any user interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Ask,
    DoNotAsk,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    Boot(boot::Command),
    Cache(cache::Command),
    Index(index::Command),
    Package(package::Command),
    Repo(repo::Command),
    Search(search::Command),
    SearchFile(search_file::Command),
    State(state::Command),
    Sync(sync::Command),
    Version(version::Command),
}

impl From<bool> for Confirmation {
    fn from(value: bool) -> Self {
        if value {
            Confirmation::DoNotAsk
        } else {
            Confirmation::Ask
        }
    }
}

impl From<&str> for Confirmation {
    fn from(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "y" | "yes" | "true" => Confirmation::DoNotAsk,
            _ => Confirmation::Ask,
        }
    }
}

fn print_system_model_warning(installation: &Installation, first_line_only: bool) {
    let path = installation.system_model_path();

    eprintln!("{}: {path:?} is present & therefore active.", "INFO".green());

    if !first_line_only {
        eprintln!(
            "Hence:
- The system-model is the source of truth and defines all
  repositories & installed packages.
- Any changes made via `moss` commands will be temporary
  until the system-model is updated.
- The system state can be reverted to match the system-model state
  by doing a `moss sync`.
- To disable the system-model, remove or rename {path:?}.",
        );
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("cache")]
    Cache(#[from] cache::Error),

    #[error(transparent)]
    Client(#[from] client::Error),

    #[error("index")]
    Index(#[from] client::index::Error),

    #[error(transparent)]
    Package(#[from] package::Error),

    #[error("repo")]
    Repo(#[from] repo::Error),

    #[error("search")]
    Search(#[from] search::Error),

    #[error("search-file")]
    SearchFile(#[from] search_file::Error),

    #[error("state")]
    State(#[from] state::Error),

    #[error("installation")]
    Installation(#[from] installation::Error),

    #[error("I/O error")]
    Io(#[from] io::Error),
}
