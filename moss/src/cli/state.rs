// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{cmp::Ordering, collections::HashMap};

use chrono::{Local, Utc};
use clap::{ArgAction, ArgMatches, Command, arg};
use moss::{
    Installation, State,
    client::{self, Client, prune},
    environment, package,
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

    let filtered: Vec<_> = state_a
        .selections
        .iter()
        .filter(|x| !state_b.selections.contains(x))
        .cloned()
        .collect();

    let diff_state = State {
        id: id_a.into(),
        summary: Some("Dummy difference state".to_string()),
        description: Some(format!("Contains selections that are in #{id_a} but not in #{id_b}")),
        selections: filtered,
        created: Utc::now(),
        kind: Kind::Transaction,
    };

    print_state_selections(diff_state, &client, Some(state_b));

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
                    release: pkg.meta.source_release,
                },
                explicit: s.explicit,
            })
        })
        .collect();

    let max_length = set.iter().map(Format::size).max().unwrap_or_default() + 2;

    let other_packages: HashMap<String, Revision> = if let Some(other) = other_state {
        print_selections_header(state.id.into(), other.id.into(), max_length);

        other
            .selections
            .into_iter()
            .filter_map(|s| {
                client.registry.by_id(&s.package).next().map(|pkg| {
                    let name = pkg.meta.name.to_string();
                    let revision = Revision {
                        version: pkg.meta.version_identifier,
                        release: pkg.meta.source_release,
                    };
                    (name, revision)
                })
            })
            .collect()
    } else {
        HashMap::new()
    };

    for item in set.clone() {
        let width = max_length - item.size() + 2;
        let name = if item.explicit {
            item.name.clone().bold()
        } else {
            item.name.clone().dim()
        };
        print!("{name} {:width$} ", " ");

        if let Some(other_revision) = other_packages.get(&item.name) {
            let latest_release = client
                .registry
                .by_name(&item.name.into(), package::Flags::new().with_available())
                .next()
                .map(|r| r.meta.source_release)
                .unwrap_or(0);

            // Determine version status and comparison
            let status = match (
                latest_release == item.revision.release,
                latest_release == other_revision.release,
            ) {
                (true, false) => VersionStatus::Latest,
                (false, true) => VersionStatus::Other,
                (true, true) => VersionStatus::Equal,
                _ => VersionStatus::Outdated,
            };

            let comparison = if item.revision.release > other_revision.release {
                ComparisonResult::Greater
            } else if item.revision.release < other_revision.release {
                ComparisonResult::Less
            } else {
                ComparisonResult::Equal
            };

            format_version_comparison(
                &item.revision.version,
                item.revision.release,
                &other_revision.version,
                other_revision.release,
                comparison,
                status,
            );
            println!();
        } else {
            println!(
                "{}-{}   {}",
                item.revision.version.magenta(),
                item.revision.release.to_string().dim(),
                "(n/a)".dim()
            );
        }
    }

    println!();
}

fn print_selections_header(id_a: i32, id_b: i32, pkg_width: usize) {
    let b_width = id_b.to_string().len();
    let a_width = id_a.to_string().len();

    let symbol = match id_a.cmp(&id_b) {
        Ordering::Greater => ">",
        Ordering::Less => "<",
        Ordering::Equal => "|",
    };

    let packages_header = format!("{:<pkg_width$}", "Packages").bold();
    let id_b_header = format!("{:>b_width$}", id_b).bold();
    let id_a_header = format!("{:>a_width$}", id_a).bold();

    println!("{} #{} {} #{}", packages_header, id_a_header, symbol, id_b_header);
    println!();
}

fn get_version_colors(status: VersionStatus) -> (&'static str, &'static str) {
    match status {
        VersionStatus::Latest => ("green", "yellow"),
        VersionStatus::Other => ("yellow", "green"),
        VersionStatus::Outdated => ("yellow", "magenta"),
        VersionStatus::Equal => ("green", "green"),
    }
}

fn format_version_comparison(
    current_version: &str,
    current_release: u64,
    other_version: &str,
    other_release: u64,
    comparison: ComparisonResult,
    status: VersionStatus,
) {
    let (current_color, other_color) = get_version_colors(status);

    let current_formatted = match current_color {
        "green" => format!("{}-{}", current_version.green(), current_release.to_string().dim()),
        "yellow" => format!("{}-{}", current_version.yellow(), current_release.to_string().dim()),
        _ => format!("{}-{}", current_version.magenta(), current_release.to_string().dim()),
    };

    let other_formatted = match other_color {
        "green" => format!("{}-{}", other_version.green(), other_release.to_string().dim()),
        "yellow" => format!("{}-{}", other_version.yellow(), other_release.to_string().dim()),
        _ => format!("{}-{}", other_version.magenta(), other_release.to_string().dim()),
    };

    let operator = match comparison {
        ComparisonResult::Greater => " > ",
        ComparisonResult::Less => " < ",
        ComparisonResult::Equal => " ~ ",
    };

    print!("{}{}{}", current_formatted, operator, other_formatted);
}

#[derive(Debug, Clone, Copy)]
enum VersionStatus {
    Latest,   // This version is the latest available
    Other,    // The other version is the latest available
    Outdated, // Neither version is the latest available
    Equal,    // Equal versions but different hashes
}

#[derive(Debug, Clone, Copy)]
enum ComparisonResult {
    Greater,
    Less,
    Equal,
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
    release: u64,
}

impl Revision {
    fn size(&self) -> usize {
        self.version.len() + self.release.to_string().len()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("db")]
    DB(#[from] moss::db::Error),
}
