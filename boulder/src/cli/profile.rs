// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, io};

use clap::Parser;
use itertools::Itertools;
use thiserror::Error;
use url::Url;

use boulder::{Env, Profile, profile};
use moss::{Installation, Repository, repository, runtime};

#[derive(Debug, Parser)]
#[command(about = "Manage boulder profiles")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    #[command(about = "List profiles")]
    List,
    #[command(about = "Add a new profile")]
    Add {
        #[arg(help = "profile name")]
        name: String,
        #[arg(
        short = 'r',
        long = "repo",
        required = true,
        help = "profile repositories",
        value_parser = parse_repository,
        help = concat!(
            "Repositories to add to profile.\n",
            "It accepts a space-separated list of repository properties.\n",
            "Each property is then separated by a comma.\n",
            "\"name\" and \"uri\" are mandatory properties.\n\n",
            "Example: --repo name=volatile,uri=https://cdn.aerynos.dev/unstable/x86_64/stone.index,priority=100")
        )]
        repos: Vec<(repository::Id, Repository)>,
    },
    #[command(about = "Update a profiles repositories")]
    Update {
        #[arg(short, long, default_value = "default-x86_64")]
        profile: profile::Id,
    },
}

/// Parse a single key-value pair
fn parse_repository(s: &str) -> Result<(repository::Id, Repository), String> {
    let key_values = s
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .collect::<BTreeMap<_, _>>();

    let id = repository::Id::new(key_values.get("name").ok_or("missing name")?);
    let uri = key_values
        .get("uri")
        .ok_or("missing uri")?
        .parse::<Url>()
        .map_err(|e| e.to_string())?;
    let priority = key_values
        .get("priority")
        .map(|p| p.parse::<u64>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();

    Ok((
        id,
        Repository {
            description: String::default(),
            uri,
            priority: repository::Priority::new(priority),
            active: true,
        },
    ))
}

pub fn handle(command: Command, env: Env) -> Result<(), Error> {
    let manager = profile::Manager::new(&env);

    match command.subcommand {
        Subcommand::List => list(manager),
        Subcommand::Add { name, repos } => add(&env, manager, name, repos),
        Subcommand::Update { profile } => update(&env, manager, &profile),
    }
}

pub fn list(manager: profile::Manager<'_>) -> Result<(), Error> {
    if manager.profiles.is_empty() {
        println!("No profiles have been configured yet");
        return Ok(());
    }

    for (id, profile) in manager.profiles.iter() {
        println!("{id}:");

        for (id, repo) in profile
            .repositories
            .iter()
            .sorted_by(|(_, a), (_, b)| a.priority.cmp(&b.priority).reverse())
        {
            println!(" - {id} = {} [{}]", repo.uri, repo.priority);
        }
    }

    Ok(())
}

pub fn add<'a>(
    env: &'a Env,
    mut manager: profile::Manager<'a>,
    name: String,
    repos: Vec<(repository::Id, Repository)>,
) -> Result<(), Error> {
    let id = profile::Id::new(&name);

    manager.save_profile(
        id.clone(),
        Profile {
            repositories: repository::Map::with(repos),
        },
    )?;

    update(env, manager, &id)?;

    println!("Profile \"{id}\" has been added");

    Ok(())
}

pub fn update<'a>(env: &'a Env, manager: profile::Manager<'a>, profile: &profile::Id) -> Result<(), Error> {
    let repos = manager.repositories(profile)?.clone();

    let installation = Installation::open(&env.moss_dir, None)?;
    let mut moss_client = moss::Client::with_explicit_repositories("boulder", installation, repos)?;
    runtime::block_on(moss_client.refresh_repositories())?;

    println!("Profile {profile} updated");

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("config")]
    Config(#[from] config::SaveError),
    #[error("profile")]
    Profile(#[from] profile::Error),
    #[error("moss client")]
    MossClient(#[from] moss::client::Error),
    #[error("moss installation")]
    MossInstallation(#[from] moss::installation::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
