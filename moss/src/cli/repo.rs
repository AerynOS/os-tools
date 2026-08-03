// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, process};

use clap::Parser;
use itertools::Itertools;
use moss::{Installation, Repository, environment, repository, runtime, system_model};
use thiserror::Error;
use tui::Styled;
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = "repo",
    about = "Manage the available software repositories visible to the installed system"
)]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

impl Command {
    /// Handles the "repo" subcommand.
    pub fn handle(self, installation: Installation) -> Result<(), Error> {
        let config = config::Manager::system(&installation.root, "moss");
        let system_model = system_model::load(&installation.system_model_path())?;
        let manager = if let Some(system_model) = &system_model {
            repository::Manager::with_system_model(environment::NAME, system_model.clone(), installation.clone())?
        } else {
            repository::Manager::with_config_manager(config, installation.clone())?
        };

        match self.subcommand {
            Subcommand::List => list(manager),
            Subcommand::Add(args) => add(manager, args),
            Subcommand::Remove { name } => remove(manager, name),
            Subcommand::Update { name } => update(manager, name),
            Subcommand::Enable { name } => enable(manager, name),
            Subcommand::Disable { name } => disable(manager, name),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("repo manager")]
    RepositoryManager(#[from] repository::manager::Error),
    #[error("load system model")]
    LoadSystemModel(#[from] system_model::LoadError),
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    #[command(visible_alias("rad"))]
    Add(AddArgs),

    #[command(visible_alias("rls"), about = "List system software repositories")]
    List,

    #[command(visible_alias("rrm"), about = "Remove a repository for the system")]
    Remove { name: String },

    #[command(visible_alias("rup"), about = "Update the system repositories")]
    Update {
        #[arg(help = "Repository to update. If not provided, all will be updated")]
        name: Option<String>,
    },

    #[command(visible_alias("ren"), about = "Enable a system repository")]
    Enable { name: String },

    #[command(visible_alias("rdi"), about = "Disable a system repository")]
    Disable { name: String },
}

#[derive(Debug, clap::Args)]
struct AddArgs {
    name: String,

    url: Url,

    #[arg(short, long)]
    comment: String,

    #[arg(short, long, help = "Repository priority", default_value_t = 0)]
    priority: u64,

    // TODO: Completely overhaul this CLI API, this is temporary to add support
    // initially for adding the new root index repo source without breaking
    // the current API.
    #[arg(
        long,
        num_args = 1,
        value_name = "root-index-options",
        value_parser = parse_root_index_options,
        help_heading = "Root index",
        help = concat!(
            "Defines the repo via root index options where <URI> is the base-uri ",
            "and all other options are passed to this flag\n\n",
            "Example: --root-index version=stream/unstable\n",
            "Example: --root-index channel=testing,version=tag/some-bug"
    ))]
    root_index: Option<RootIndexOptions>,
}

// Actual implementation of moss repo add
fn add(mut manager: repository::Manager, args: AddArgs) -> Result<(), Error> {
    let id = repository::Id::new(&args.name);

    let source = if let Some(RootIndexOptions { channel, version, arch }) = args.root_index {
        repository::Source::RootIndex(repository::RootIndexSource {
            base_uri: args.url,
            channel,
            version,
            arch,
        })
    } else {
        repository::Source::DirectIndex(args.url)
    };

    manager.add_repository(
        id.clone(),
        Repository {
            description: args.comment,
            source,
            priority: args.priority.into(),
            active: true,
        },
    )?;

    runtime::block_on(manager.refresh(&id))?;

    println!("{id} added");

    Ok(())
}

/// List the repositories and pretty print them
fn list(manager: repository::Manager) -> Result<(), Error> {
    let configured_repos = manager.list();
    if configured_repos.len() == 0 {
        println!("No repositories have been configured yet");
        return Ok(());
    }

    for (id, repo) in configured_repos.sorted_by(|(_, a), (_, b)| a.priority.cmp(&b.priority).reverse()) {
        let disabled = if !repo.active {
            " (disabled)".dim().to_string()
        } else {
            String::new()
        };

        // TODO: Refactor this in future unit of work to print KDL encoded
        // documents for each repo. The below addition of `RootIndexSource`
        // is a temporary fix, not the desired future state
        match &repo.source {
            repository::Source::DirectIndex(uri) => println!(" - {id} = {uri} [{}]{disabled}", repo.priority),
            repository::Source::RootIndex(repository::RootIndexSource {
                base_uri,
                channel,
                version,
                arch,
            }) => println!(
                " - {id} = (base-uri={base_uri}, channel={channel}, version={version}, arch={arch}) [{}]{disabled}",
                repo.priority
            ),
        }
    }

    Ok(())
}

/// Update specific repos or all
fn update(manager: repository::Manager, which: Option<String>) -> Result<(), Error> {
    runtime::block_on(async {
        match which {
            Some(repo) => manager.refresh(&repository::Id::new(&repo)).await,
            None => manager.refresh_all().await,
        }
    })?;

    Ok(())
}

/// Remove repo
fn remove(mut manager: repository::Manager, repo: String) -> Result<(), Error> {
    let id = repository::Id::new(&repo);

    match manager.remove(id.clone())? {
        repository::manager::Removal::NotFound => {
            println!("{id} not found");
            process::exit(1);
        }
        repository::manager::Removal::ConfigDeleted(false) => {
            println!(
                "{id} configuration must be manually deleted since it doesn't exist in it's own configuration file"
            );
            process::exit(1);
        }
        repository::manager::Removal::ConfigDeleted(true) => {
            println!("{id} removed");
        }
    }

    Ok(())
}

fn enable(mut manager: repository::Manager, repo: String) -> Result<(), Error> {
    let id = repository::Id::new(&repo);

    runtime::block_on(manager.enable(&id))?;

    println!("{id} enabled");

    Ok(())
}

fn disable(mut manager: repository::Manager, repo: String) -> Result<(), Error> {
    let id = repository::Id::new(&repo);

    runtime::block_on(manager.disable(&id))?;

    println!("{id} disabled");

    Ok(())
}

#[derive(Debug, Clone)]
struct RootIndexOptions {
    channel: repository::format::Identifier,
    version: repository::format::ScopedIdentifier,
    arch: String,
}

fn parse_root_index_options(s: &str) -> Result<RootIndexOptions, String> {
    if !s.contains('=') {
        return Err("options must be key=value[,key=value]*".to_owned());
    }

    let mut key_values = s
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .collect::<BTreeMap<_, _>>();

    let channel =
        repository::format::Identifier::try_from(key_values.remove("channel").unwrap_or(repository::DEFAULT_CHANNEL))
            .map_err(|err| err.to_string())?;
    let version = key_values
        .remove("version")
        .ok_or("version is required")?
        .parse::<repository::format::ScopedIdentifier>()
        .map_err(|err| format!("invalid version identifier: {err}"))?;
    let arch = key_values.remove("arch").unwrap_or(repository::DEFAULT_ARCH).to_owned();

    if let Some(key) = key_values.into_keys().next() {
        return Err(format!("unknown key: {key}"));
    }

    Ok(RootIndexOptions { channel, version, arch })
}
