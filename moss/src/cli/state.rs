// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::HashMap;

use chrono::{Local, Utc};
use clap::{ArgAction, ArgMatches, Command, arg};
use itertools::Itertools;
use moss::{
    Installation, State,
    client::{self, Client, prune},
    environment,
    state::Kind,
};
use thiserror::Error;
use tui::Styled;

pub fn command() -> Command {
    Command::new("state")
        .about("Manage state")
        .long_about("Manage state ...")
        .subcommand_required(true)
        .subcommand(Command::new("active").about("List the active state"))
        .subcommand(Command::new("list").about("List all states"))
        .subcommand(
            Command::new("activate")
                .about("Activate a state")
                .arg(
                    arg!(<ID> "State id to be activated")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(arg!(--"skip-triggers" "Do not run triggers on activation").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("diff")
                .about("Query difference between two states")
                .arg(
                    arg!(<A> "Old state id to query")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    arg!(<B> "New state id to query")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64)),
                ),
        )
        .subcommand(
            Command::new("query").about("Query information for a state").arg(
                arg!(<ID> "State id to query")
                    .action(ArgAction::Set)
                    .value_parser(clap::value_parser!(u64)),
            ),
        )
        .subcommand(
            Command::new("prune")
                .about("Prune archived states")
                .arg(
                    arg!(-k --keep "Keep this many states")
                        .action(ArgAction::Set)
                        .default_value("10")
                        .value_parser(clap::value_parser!(u64).range(1..)),
                )
                .arg(
                    arg!(--"include-newer" "Include states newer than the active state when pruning")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("remove").about("Remove an archived state").arg(
                arg!(<ID> "State id to be removed")
                    .action(ArgAction::Set)
                    .value_parser(clap::value_parser!(u64)),
            ),
        )
        .subcommand(
            Command::new("verify")
                .about("Verify TODO")
                .arg(arg!(--verbose "Vebose output").action(ArgAction::SetTrue)),
        )
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    match args.subcommand() {
        Some(("active", _)) => active(installation),
        Some(("list", _)) => list(installation),
        Some(("activate", args)) => activate(args, installation),
        Some(("diff", args)) => diff(args, installation),
        Some(("query", args)) => query(args, installation),
        Some(("prune", args)) => prune(args, installation),
        Some(("remove", args)) => remove(args, installation),
        Some(("verify", args)) => verify(args, installation),
        _ => unreachable!(),
    }
}

/// List the active state
pub fn active(installation: Installation) -> Result<(), Error> {
    if let Some(id) = installation.active_state {
        let client = Client::new(environment::NAME, installation)?;

        let state = client.state_db.get(id)?;

        print_state(state);
    }

    Ok(())
}

/// List all known states, newest first
pub fn list(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;

    let state_ids = client.state_db.list_ids()?;

    let mut states = state_ids
        .into_iter()
        .map(|(id, _)| client.state_db.get(id).map_err(Error::DB))
        .collect::<Result<Vec<_>, _>>()?;

    states.reverse();
    states.into_iter().for_each(print_state);
    Ok(())
}

pub fn activate(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let new_id = *args.get_one::<u64>("ID").unwrap() as i32;
    let skip_triggers = args.get_flag("skip-triggers");

    let client = Client::new(environment::NAME, installation)?;
    let old_id = client.activate_state(new_id.into(), skip_triggers)?;

    println!(
        "State {} activated {}",
        new_id.to_string().bold(),
        format!("({old_id} archived)").dim()
    );

    Ok(())
}

pub fn diff(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let id_a = *args.get_one::<u64>("A").unwrap() as i32;
    let id_b = *args.get_one::<u64>("B").unwrap() as i32;

    let client = Client::new(environment::NAME, installation)?;

    let state_a = client.state_db.get(id_a.into())?;
    let state_b = client.state_db.get(id_b.into())?;

    let filtered: Vec<_> = state_b
        .selections
        .iter()
        .filter(|x| !state_a.selections.contains(x))
        .cloned()
        .collect();

    let diff_state = State {
        id: 0.into(),
        summary: Some("Dummy difference state".to_string()),
        description: Some(format!("Contains selections that are in #{id_b} but not in #{id_a}")),
        selections: filtered,
        created: Utc::now(),
        kind: Kind::Transaction,
    };

    print_state(diff_state.clone());
    print_state_selections(diff_state, &client, Some(state_a));

    Ok(())
}

pub fn query(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let id = *args.get_one::<u64>("ID").unwrap() as i32;

    let client = Client::new(environment::NAME, installation)?;

    let state = client.state_db.get(id.into())?;

    print_state(state.clone());
    print_state_selections(state, &client, None);

    Ok(())
}

pub fn prune(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keep = *args.get_one::<u64>("keep").unwrap();
    let include_newer = args.get_flag("include-newer");
    let yes = args.get_flag("yes");

    let client = Client::new(environment::NAME, installation)?;
    client.prune(prune::Strategy::KeepRecent { keep, include_newer }, yes)?;

    Ok(())
}

pub fn remove(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let id = *args.get_one::<u64>("ID").unwrap() as i32;
    let yes = args.get_flag("yes");

    let client = Client::new(environment::NAME, installation)?;
    client.prune(prune::Strategy::Remove(id.into()), yes)?;

    Ok(())
}

pub fn verify(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let verbose = args.get_flag("verbose");
    let yes = args.get_flag("yes");

    let client = Client::new(environment::NAME, installation)?;
    client.verify(yes, verbose)?;

    Ok(())
}

/// Emit a state description for the TUI
fn print_state(state: State) {
    let local_time = state.created.with_timezone(&Local);
    let formatted_time = local_time.format("%Y-%m-%d %H:%M:%S %Z");

    println!(
        "State #{} - {}",
        state.id.to_string().bold(),
        state.summary.unwrap_or_else(|| String::from("system transaction"))
    );
    println!("{} {formatted_time}", "Created:".bold());
    if let Some(desc) = &state.description {
        println!("{} {desc}", "Description:".bold());
    }
    println!("{} {}", "Packages:".bold(), state.selections.len());
    println!();
}

fn print_state_selections(state: State, client: &Client, other_state: Option<State>) {
    let set: Vec<_> = state
        .selections
        .into_iter()
        .filter_map(|s| {
            client.registry.by_id(&s.package).next().map(|pkg| Format {
                name: pkg.meta.name.to_string(),
                revision: Revision {
                    version: pkg.meta.version_identifier,
                    release: pkg.meta.source_release.to_string(),
                },
                explicit: s.explicit,
            })
        })
        .collect();

    let other_packages: HashMap<String, Revision> = if let Some(other) = other_state {
        other
            .selections
            .into_iter()
            .filter_map(|s| {
                client.registry.by_id(&s.package).next().map(|pkg| {
                    let name = pkg.meta.name.to_string();
                    let revision = Revision {
                        version: pkg.meta.version_identifier,
                        release: pkg.meta.source_release.to_string(),
                    };
                    (name, revision)
                })
            })
            .collect()
    } else {
        HashMap::new()
    };

    let max_length = set.iter().map(Format::size).max().unwrap_or_default() + 2;

    for item in set.clone() {
        let width = max_length - item.size() + 2;
        let name = if item.explicit {
            item.name.clone().bold()
        } else {
            item.name.clone().dim()
        };
        print!("{name} {:width$} ", " ");

        // Check if we have version comparison data
        if let Some(other_revision) = other_packages.get(&item.name) {
            // Print current version
            print!(
                "{}-{}",
                item.revision.version.clone().magenta(),
                item.revision.release.clone().dim()
            );

            // Compare versions and show difference
            if item.revision.release != other_revision.release {
                print!(
                    " (was {}-{})",
                    other_revision.version.clone().yellow(),
                    other_revision.release.clone().dim()
                );
            }
            println!();
        } else {
            // No comparison available, just print current version
            println!("{}-{}", item.revision.version.magenta(), item.revision.release.dim());
        }
    }

    println!();
}

#[derive(Clone, Debug)]
struct Format {
    name: String,
    revision: Revision,
    explicit: bool,
}

impl Format {
    fn size(&self) -> usize {
        self.name.len() + self.revision.size()
    }
}

#[derive(Clone, Debug)]
struct Revision {
    version: String,
    release: String,
}

impl Revision {
    fn size(&self) -> usize {
        self.version.len() + self.release.len()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("db")]
    DB(#[from] moss::db::Error),
}
