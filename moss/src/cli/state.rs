// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashSet,
    io,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

use chrono::Local;
use clap::Parser;
use fs_err as fs;
use moss::{
    Installation, State,
    client::{self, Client, prune},
    environment, state,
};
use nix::unistd::gethostname;
use thiserror::Error;
use tui::Styled;

use crate::cli::{Confirmation, Global};

#[derive(Debug, Parser)]
#[command(
    name = "state",
    about = "Manage the available software repositories visible to the installed system"
)]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

impl Command {
    pub fn handle(self, global: Global, installation: Installation) -> Result<(), Error> {
        match self.subcommand {
            Subcommand::Activate(args) => activate(args, installation)?,
            Subcommand::List(args) => list(global, args, installation)?,
            Subcommand::Search(args) => search(global, args, installation)?,
            Subcommand::Info { id } => info(id, installation)?,
            Subcommand::Verify => verify(global, installation)?,
            Subcommand::Export { id, output } => export(id, output, installation)?,
            Subcommand::Remove(args) => remove(global, args, installation)?,
            Subcommand::BuildVfs => build_vfs(installation)?,
        }
        Ok(())
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
    #[error("invalid state id or range: {0}")]
    InvalidRange(String),
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    #[command(visible_alias("sac"), about = "Activate the given valid state")]
    Activate(ActivateArgs),

    #[command(visible_alias("stl"), about = "List all states")]
    List(ListArgs),

    #[command(visible_alias = "sts")]
    Search(SearchArgs),

    #[command(visible_alias = "sti", about = "Show information about a state")]
    Info {
        #[arg(help = "State ID. If \"active\" is passed, show the currently active state")]
        id: String,
    },

    #[command(visible_alias("sve"), about = "Verify and fix system states and assets")]
    Verify,

    #[command(visible_alias("ste"), about = "Export a state as a system-model.kdl file")]
    Export {
        /// State id to export or current state if omitted.
        id: Option<i32>,

        /// Export to the provided path or stdout if not supplied.
        ///
        /// If supplied without a path or path is a directory,
        /// outputs to "system-model-{hostname}-fstxn-{id}.kdl".
        #[arg(short, long)]
        output: Option<Option<PathBuf>>,
    },

    #[command(
        visible_alias("srm"),
        about = "Remove arbitrary states. Supports single states and inclusive ranges a-b"
    )]
    Remove(RemoveArgs),

    // For profiling only, hence hidden.
    //
    // Builds a VFS of the currently-active state, and throws it away again.
    // Run this through hyperfine / valgrind / heaptrack to profile the VFS
    // code.
    #[command(hide = true)]
    BuildVfs,
}

#[derive(Debug, Clone, clap::Args)]
struct ActivateArgs {
    state: i32,

    #[arg(long, help = "Do not run triggers on activation")]
    skip_triggers: bool,

    #[arg(long, help = "Do not sync boot on activation")]
    skip_boot: bool,
}

#[derive(Debug, Clone, clap::Args)]
struct ListArgs {
    #[arg(short = 's', long = "sync", help = "Reduce the output to synced states")]
    filter_synced: bool,

    #[arg(short = 'a', long = "activate", help = "Reduce the output to activated states")]
    filter_activated: bool,

    #[arg(short = 'r', long = "remove", help = "Reduce the output to removed states")]
    filter_removed: bool,
}

#[derive(Clone, Debug, clap::Args)]
struct SearchArgs {
    #[arg(help = "State ID to show. If \"active\" is passed, show the currently active state")]
    id: Option<String>,

    #[arg(
        short = 's',
        long = "sync",
        conflicts_with = "id",
        help = "Reduce the output to synced states"
    )]
    filter_synced: bool,

    #[arg(
        short = 'a',
        long = "activate",
        conflicts_with = "id",
        help = "Reduce the output to activated states"
    )]
    filter_activated: bool,

    #[arg(
        short = 'r',
        long = "remove",
        conflicts_with = "id",
        help = "Reduce the output to removed states"
    )]
    filter_removed: bool,
}

// These args are mutually exclusive. We would use a enum for that,
// but clap does not support this feature: we must pass "exclusive=true"
// to all args manually. See
// https://users.rust-lang.org/t/mutually-exclusive-command-line-arguments-with-clap-or-another-library/134091
#[derive(Debug, Clone, clap::Args)]
struct RemoveArgs {
    #[arg(num_args=1.., exclusive = true, value_parser = parse_state_list)]
    states: Vec<RangeInclusive<i32>>,

    #[arg(short, long, exclusive = true, help = "Remove all but the n latest states")]
    keep: Option<u32>,

    #[arg(short, long, exclusive = true, help = "Remove all but the currently bootable states")]
    prune: bool,
}

fn parse_state_list(s: &str) -> Result<RangeInclusive<i32>, String> {
    if let Some((start, end)) = s.split_once('-') {
        let start = start.parse::<i32>().map_err(|_| "invalid start of range")?;
        let end = end.parse::<i32>().map_err(|_| "invalid end of range")?;
        let range = start..=end;
        if range.is_empty() {
            return Err("invalid range of IDs".to_owned());
        }
        Ok(range)
    } else {
        let id = s.parse::<i32>().map_err(|_| "invalid number")?;
        Ok(id..=id)
    }
}

fn activate(args: ActivateArgs, installation: Installation) -> Result<(), Error> {
    let new_id = state::Id::from(args.state);

    let client = Client::new(environment::NAME, installation)?;
    let old_id = client.activate_state(new_id, args.skip_triggers, args.skip_boot)?;

    println!(
        "State {} activated {}",
        new_id.to_string().bold(),
        format!("({old_id} archived)").dim()
    );

    Ok(())
}

/// List all known states, newest first
fn list(global: Global, args: ListArgs, installation: Installation) -> Result<(), Error> {
    if args.filter_activated || args.filter_synced || args.filter_removed {
        unimplemented!("Filtered arguments not yet implemented");
    }

    let client = Client::new(environment::NAME, installation)?;

    if let Some(state) = client.get_active_state()? {
        print_state(state.clone());
        if global.verbose {
            print_state_selections(state, &client)?;
        }
    }
    for state in client.list_states()?.into_iter().rev() {
        print_state(state.clone());
        if global.verbose {
            print_state_selections(state, &client)?;
        }
    }

    Ok(())
}

fn search(_global: Global, _args: SearchArgs, _installation: Installation) -> Result<(), Error> {
    unimplemented!("searching states is not yet implemented");
}

fn info(id: String, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;
    if id.to_lowercase() == "active" {
        if let Some(state) = client.get_active_state()? {
            print_state(state.clone());
            print_state_selections(state, &client)?;
        }
    } else {
        let id = id.parse::<i32>().map_err(|_| Error::InvalidRange(id.to_owned()))?;
        let state = client.get_state(id.into())?;
        print_state(state.clone());
        print_state_selections(state, &client)?;
    }
    Ok(())
}

fn verify(global: Global, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;
    client.verify(global.confirm == Confirmation::DoNotAsk, global.verbose)?;
    Ok(())
}

fn export(id: Option<i32>, output: Option<Option<PathBuf>>, installation: Installation) -> Result<(), Error> {
    let id = match id {
        Some(id) => state::Id::from(id),
        None => installation.active_state.ok_or(Error::NoActiveState)?,
    };

    let client = Client::new(environment::NAME, installation)?;
    let system_model = client.export_state(id)?;

    match output {
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

fn remove(global: Global, args: RemoveArgs, installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;

    if !args.states.is_empty() {
        let mut ids = HashSet::new();
        for range in args.states {
            ids.extend(range.map(state::Id::from));
        }
        let ids = ids.into_iter().collect::<Vec<_>>();
        client.prune_states(prune::Strategy::Remove(&ids), global.confirm == Confirmation::DoNotAsk)?;
    } else if let Some(num) = args.keep {
        client.prune_states(
            prune::Strategy::KeepRecent {
                keep: num as u64,
                include_newer: false,
            },
            global.confirm == Confirmation::DoNotAsk,
        )?;
    } else if args.prune {
        unimplemented!("Pruning all states but bootable ones is not yet implemented");
    }

    unreachable!();
}

fn build_vfs(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation)?;

    if let Some(state) = client.get_active_state()? {
        let fstree = client.vfs(state.selections.iter().map(|selection| &selection.package))?;

        std::hint::black_box(fstree);
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

fn print_state_selections(state: State, client: &Client) -> Result<(), Error> {
    let set = state
        .selections
        .into_iter()
        .map(|s| {
            let pkg = client.resolve_package(&s.package)?;

            Ok(Format {
                name: pkg.meta.name.to_string(),
                revision: Revision {
                    version: pkg.meta.version_identifier,
                    release: pkg.meta.source_release,
                },
                explicit: s.explicit,
            })
        })
        .collect::<Result<Vec<_>, client::Error>>()?;

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

    Ok(())
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
