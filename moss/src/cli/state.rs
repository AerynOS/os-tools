// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{cmp::Ordering, collections::HashMap};

use chrono::Local;
use clap::{ArgAction, ArgMatches, Command, arg};
use moss::{
    Installation, State,
    client::{self, Client, prune},
    environment,
    package::{self, Id},
    state::Selection,
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

    print_diff_selections(state_a, &client, state_b);

    Ok(())
}

pub fn query(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let id = *args.get_one::<u64>("ID").unwrap() as i32;

    let client = Client::new(environment::NAME, installation)?;

    let state = client.state_db.get(id.into())?;

    print_state(state.clone());
    print_state_selections(state, &client);

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

fn print_state_selections(state: State, client: &Client) {
    let set: Vec<_> = state
        .selections
        .into_iter()
        .filter_map(|s| {
            client.registry.by_id(&s.package).next().map(|pkg| Format {
                name: pkg.meta.name.to_string(),
                id: pkg.id,
                revision: Revision {
                    version: pkg.meta.version_identifier,
                    release: pkg.meta.source_release,
                },
                explicit: s.explicit,
            })
        })
        .collect();

    let max_length = set.iter().map(Format::size).max().unwrap_or_default() + 2;

    for item in set.clone() {
        let width = max_length - item.size() + 2;
        let name = if item.explicit {
            item.name.clone().bold()
        } else {
            item.name.clone().dim()
        };
        print!("{name} {:width$} ", " ");
        println!(
            "{}-{}",
            item.revision.version.magenta(),
            item.revision.release.to_string().dim(),
        );
    }
    println!();
}

fn print_diff_selections(state_a: State, client: &Client, state_b: State) {
    let selections_to_map = |s: Selection| {
        client.registry.by_id(&s.package).next().map(|pkg| {
            let name = pkg.meta.name.to_string();
            let format = Format {
                name: name.clone(),
                id: pkg.id,
                revision: Revision {
                    version: pkg.meta.version_identifier,
                    release: pkg.meta.source_release,
                },
                explicit: s.explicit,
            };
            (name, format)
        })
    };

    let pkgs_a: HashMap<String, Format> = state_a.selections.into_iter().filter_map(selections_to_map).collect();
    let pkgs_b: HashMap<String, Format> = state_b.selections.into_iter().filter_map(selections_to_map).collect();

    let mut all_names = pkgs_a.keys().collect::<Vec<_>>();
    all_names.extend(pkgs_b.keys().collect::<Vec<_>>());

    let max_length = all_names
        .iter()
        .map(|s| {
            std::cmp::max(
                pkgs_a.get(s.to_owned()).map(|e| e.size()).unwrap_or_default(),
                pkgs_b.get(s.to_owned()).map(|e| e.size()).unwrap_or_default(),
            )
        })
        .max()
        .unwrap_or_default()
        + 2;

    print_selections_header(state_a.id.into(), state_b.id.into(), max_length);

    // Iterate through all unique package names and print the differences
    for name in all_names {
        let width = max_length
            - std::cmp::max(
                pkgs_a.get(&name.to_owned()).map(|e| e.size()).unwrap_or_default(),
                pkgs_b.get(&name.to_owned()).map(|e| e.size()).unwrap_or_default(),
            );
        let explicit = pkgs_a.get(&name.to_owned()).is_some_and(|e| e.explicit)
            || pkgs_b.get(&name.to_owned()).is_some_and(|e| e.explicit);
        let name_styled = if explicit {
            name.clone().bold()
        } else {
            name.clone().dim()
        };

        // Get the latest available version to determine if package is up to date
        let latest_release = client
            .registry
            .by_name(&name.to_owned().into(), package::Flags::new().with_available())
            .next()
            .map(|r| r.meta.source_release)
            .unwrap_or(0);

        match (pkgs_a.get(&name.to_owned()), pkgs_b.get(&name.to_owned())) {
            // Case 1: Package is in both states. Check if it was modified.
            (Some(a), Some(b)) => {
                if a.id != b.id {
                    print!("{name_styled} {:width$} ", " ");
                    let comparison = if a.revision.release > b.revision.release {
                        ComparisonResult::Greater
                    } else if a.revision.release < b.revision.release {
                        ComparisonResult::Less
                    } else {
                        ComparisonResult::Equal
                    };

                    let status = match (
                        latest_release == a.revision.release,
                        latest_release == b.revision.release,
                    ) {
                        (true, false) => VersionStatus::Latest,
                        (false, true) => VersionStatus::Other,
                        (true, true) => VersionStatus::Equal,
                        _ => VersionStatus::Outdated,
                    };

                    format_version_comparison(a.revision.clone(), b.revision.clone(), comparison, status);
                    println!();
                }
            }
            // Case 2: Package in A, not in B
            (Some(a), None) => {
                print!("{name_styled} {:width$} ", " ");
                println!(
                    "{}-{}   {}",
                    a.revision.version.clone().magenta(),
                    a.revision.release.to_string().dim(),
                    "(n/a)".dim()
                );
            }
            // Case 3: Package in B, not in A
            (None, Some(b)) => {
                print!("{name_styled} {:width$} ", " ");
                println!(
                    "{}-{}   {}",
                    b.revision.version.clone().magenta(),
                    b.revision.release.to_string().dim(),
                    "(n/a)".dim()
                );
            }
            (None, None) => unreachable!(),
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
    let id_b_header = format!("{id_b:>b_width$}").bold();
    let id_a_header = format!("{id_a:>a_width$}").bold();

    println!("{packages_header} #{id_a_header} {symbol} #{id_b_header}");
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
    current_rev: Revision,
    other_rev: Revision,
    comparison: ComparisonResult,
    status: VersionStatus,
) {
    let (current_color, other_color) = get_version_colors(status);

    let current_formatted = match current_color {
        "green" => format!(
            "{}-{}",
            current_rev.version.green(),
            current_rev.release.to_string().dim()
        ),
        "yellow" => format!(
            "{}-{}",
            current_rev.version.yellow(),
            current_rev.release.to_string().dim()
        ),
        _ => format!(
            "{}-{}",
            current_rev.version.magenta(),
            current_rev.release.to_string().dim()
        ),
    };

    let other_formatted = match other_color {
        "green" => format!("{}-{}", other_rev.version.green(), other_rev.release.to_string().dim()),
        "yellow" => format!("{}-{}", other_rev.version.yellow(), other_rev.release.to_string().dim()),
        _ => format!(
            "{}-{}",
            other_rev.version.magenta(),
            other_rev.release.to_string().dim()
        ),
    };

    let operator = match comparison {
        ComparisonResult::Greater => " > ",
        ComparisonResult::Less => " < ",
        ComparisonResult::Equal => " ~ ",
    };

    print!("{current_formatted}{operator}{other_formatted}");
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
    id: Id,
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
