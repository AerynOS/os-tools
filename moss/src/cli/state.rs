// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
};

use chrono::Local;
use clap::{ArgAction, ArgMatches, Command, CommandFactory, FromArgMatches, Parser, arg};
use fs_err as fs;
use moss::{
    Installation, State,
    client::{self, Client, prune},
    environment, state,
};
use nix::unistd::gethostname;
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
        .subcommand(Export::command())
}

#[derive(Debug, Parser)]
#[command(name = "export", about = "Export a state as a system-model.kdl file")]
struct Export {
    /// State id to export or current state if omitted
    id: Option<i32>,
    /// Export to the provided path or stdout if not supplied
    ///
    /// If supplied without a path or path is a directory, outputs to "system-model-{hostname}-fstxn-{id}.kdl"
    #[arg(short, long)]
    output: Option<Option<PathBuf>>,
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    match args.subcommand() {
        Some(("active", _)) => active(installation),
        Some(("list", _)) => list(installation),
        Some(("activate", args)) => activate(args, installation),
        Some(("query", args)) => query(args, installation),
        Some(("prune", args)) => prune(args, installation),
        Some(("remove", args)) => remove(args, installation),
        Some(("verify", args)) => verify(args, installation),
        Some(("export", args)) => export(args, installation),
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
    client.prune_states(prune::Strategy::KeepRecent { keep, include_newer }, yes)?;

    Ok(())
}

pub fn remove(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let id = *args.get_one::<u64>("ID").unwrap() as i32;
    let yes = args.get_flag("yes");

    let client = Client::new(environment::NAME, installation)?;
    client.prune_states(prune::Strategy::Remove(id.into()), yes)?;

    Ok(())
}

pub fn verify(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let verbose = args.get_flag("verbose");
    let yes = args.get_flag("yes");

    let client = Client::new(environment::NAME, installation)?;
    client.verify(yes, verbose)?;

    Ok(())
}

fn export(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let export = Export::from_arg_matches(args).expect("validate by clap");

    let id = match export.id {
        Some(id) => state::Id::from(id),
        None => installation.active_state.ok_or(Error::NoActiveState)?,
    };

    let client = Client::new(environment::NAME, installation)?;
    let system_model = client.export_state(id)?;

    match export.output {
        Some(maybe_path) => {
            let format_filename = || {
                if let Some(hostname) = gethostname().ok().and_then(|s| s.into_string().ok()) {
                    format!("system-model-{hostname}-fstxn-{id}.kdl")
                } else {
                    format!("system-model-fstxn-{id}.kdl")
                }
            };

            let path = match maybe_path {
                Some(path) => {
                    if path.is_dir() {
                        path.join(format_filename())
                    } else {
                        path
                    }
                }
                None => Path::new(".").join(format_filename()),
            };

            fs::write(&path, system_model.encoded())?;

            println!("Exported to {path:?}");
        }
        None => {
            println!("{}", system_model.encoded());
        }
    }

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
    #[error("io")]
    Io(#[from] io::Error),
    #[error("no active state")]
    NoActiveState,
}
