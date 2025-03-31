// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use chrono::Local;
use clap::{arg, ArgAction, ArgMatches, Command};
use moss::{
    client::{self, prune, Client},
    environment, state, Installation,
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

    tracing::info!(
        "State {} activated {}",
        new_id.to_string().bold(),
        format!("({old_id} archived)").dim()
    );

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
fn print_state(state: state::State) {
    let local_time = state.created.with_timezone(&Local);
    let formatted_time = local_time.format("%Y-%m-%d %H:%M:%S %Z");

    tracing::info!(
        "State #{} - {}",
        state.id.to_string().bold(),
        state.summary.unwrap_or_else(|| String::from("system transaction"))
    );
    tracing::info!("{} {formatted_time}", "Created:".bold());
    if let Some(desc) = &state.description {
        tracing::info!("{} {desc}", "Description:".bold());
    }
    tracing::info!("{} {}", "Packages:".bold(), state.selections.len());
    println!();
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("db")]
    DB(#[from] moss::db::Error),
}
