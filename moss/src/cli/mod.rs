// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{env, fs, io, path::Path, path::PathBuf};

use clap::{Arg, ArgAction, Command};
use clap_complete::{
    generate_to,
    shells::{Bash, Fish, Zsh},
};
use clap_mangen::Man;
use moss::{Installation, installation};
use thiserror::Error;
use tracing_common::{self, logging::LogConfig, logging::init_log_with_config};
use tui::Styled;

mod boot;
mod cache;
mod extract;
mod index;
mod info;
mod inspect;
mod install;
mod list;
mod remove;
mod repo;
mod search;
mod search_file;
mod state;
mod sync;
mod version;

/// Generate the CLI command structure
fn command() -> Command {
    Command::new("moss")
        .about("Advanced system state & package manager")
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .global(true)
                .help("Prints additional information about what moss is doing")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .global(true)
                .help("Prints version information about the binary")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("root")
                .short('D')
                .long("directory")
                .global(true)
                .help("Root directory")
                .action(ArgAction::Set)
                .default_value("/")
                .value_parser(clap::value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("cache")
                .long("cache")
                .global(true)
                .help("Cache directory")
                .action(ArgAction::Set)
                .value_parser(clap::value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("log")
                .long("log")
                .help("Logging configuration: <level>[:<format>][:<destination>]\nLevels: trace, debug, info, warn, error\nFormats: text, json\nDestinations: stderr, <file>")
                .action(ArgAction::Set)
                .global(true)
                .value_parser(clap::value_parser!(LogConfig)),
        )
        .arg(
            Arg::new("yes")
                .short('y')
                .long("yes-all")
                .global(true)
                .help("Assume yes for all questions")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("generate-manpages")
                .long("generate-manpages")
                .help("Generate manpages")
                .action(ArgAction::Set)
                .value_name("DIR")
                .hide(true),
        )
        .arg(
            Arg::new("generate-completions")
                .long("generate-completions")
                .help("Generate shell completions")
                .action(ArgAction::Set)
                .value_name("DIR")
                .hide(true),
        )
        .arg_required_else_help(true)
        .subcommand(boot::command())
        .subcommand(cache::command())
        .subcommand(extract::command())
        .subcommand(index::command())
        .subcommand(info::command())
        .subcommand(inspect::command())
        .subcommand(install::command())
        .subcommand(list::command())
        .subcommand(remove::command())
        .subcommand(repo::command())
        .subcommand(search::command())
        .subcommand(search_file::command())
        .subcommand(state::command())
        .subcommand(sync::command())
        .subcommand(version::command())
}

/// Generate manpages for all commands recursively
fn generate_manpages(cmd: &Command, dir: &Path, prefix: Option<&str>) -> io::Result<()> {
    let name = cmd.get_name();
    let man = Man::new(cmd.to_owned());
    let mut buffer: Vec<u8> = Default::default();
    man.render(&mut buffer)?;

    let filename = if let Some(prefix) = prefix {
        format!("{prefix}-{name}.1")
    } else {
        format!("{name}.1")
    };

    fs::write(dir.join(filename), buffer)?;

    for subcmd in cmd.get_subcommands() {
        let new_prefix = if let Some(p) = prefix {
            format!("{p}-{name}")
        } else {
            name.to_owned()
        };
        generate_manpages(subcmd, dir, Some(&new_prefix))?;
    }
    Ok(())
}

/// Generate shell completions
fn generate_completions(cmd: &mut Command, dir: &Path) -> io::Result<()> {
    generate_to(Bash, cmd, "moss", dir)?;
    generate_to(Fish, cmd, "moss", dir)?;
    generate_to(Zsh, cmd, "moss", dir)?;
    Ok(())
}

/// Process all CLI arguments
pub fn process() -> Result<(), Error> {
    let args = replace_aliases(env::args());
    let matches = command().get_matches_from(args);

    let show_version = matches.get_one::<bool>("version").is_some_and(|v| *v);

    if show_version {
        println!("moss {}", tools_buildinfo::get_full_version());
    }

    if let Some(log_config) = matches.get_one::<LogConfig>("log") {
        init_log_with_config(log_config.clone());
    }

    if let Some(dir) = matches.get_one::<String>("generate-manpages") {
        let dir = Path::new(dir);
        fs::create_dir_all(dir)?;
        generate_manpages(&command(), dir, None)?;
        return Ok(());
    }

    if let Some(dir) = matches.get_one::<String>("generate-completions") {
        let dir = Path::new(dir);
        fs::create_dir_all(dir)?;
        generate_completions(&mut command(), dir)?;
        return Ok(());
    }

    // Print the version, but not if the user is using the version subcommand
    if matches.get_flag("verbose")
        && let Some(command) = matches.subcommand_name()
        && command != "version"
    {
        version::print();
    }

    let root = matches.get_one::<PathBuf>("root").unwrap();
    let cache = matches.get_one::<PathBuf>("cache");

    let installation = Installation::open(root, cache.cloned())?;

    if installation.system_model.is_some() {
        print_system_model_warning(&installation);
    }

    match matches.subcommand() {
        Some(("boot", args)) => boot::handle(args, installation).map_err(Error::Boot),
        Some(("cache", args)) => cache::handle(args, installation).map_err(Error::Cache),
        Some(("extract", args)) => extract::handle(args).map_err(Error::Extract),
        Some(("index", args)) => index::handle(args).map_err(Error::Index),
        Some(("info", args)) => info::handle(args, installation).map_err(Error::Info),
        Some(("inspect", args)) => inspect::handle(args).map_err(Error::Inspect),
        Some(("install", args)) => install::handle(args, installation).map_err(Error::Install),
        Some(("list", args)) => list::handle(args, installation).map_err(Error::List),
        Some(("remove", args)) => remove::handle(args, installation).map_err(Error::Remove),
        Some(("repo", args)) => repo::handle(args, installation).map_err(Error::Repo),
        Some(("search", args)) => search::handle(args, installation).map_err(Error::Search),
        Some(("search-file", args)) => search_file::handle(args, installation).map_err(Error::SearchFile),
        Some(("state", args)) => state::handle(args, installation).map_err(Error::State),
        Some(("sync", args)) => sync::handle(args, installation).map_err(Error::Sync),
        Some(("version", args)) => {
            version::handle(args);
            Ok(())
        }
        None => {
            if !show_version {
                command().print_help().unwrap();
            }
            Ok(())
        }
        _ => unreachable!(),
    }
}

fn replace_aliases(args: env::Args) -> Vec<String> {
    const ALIASES: &[(&str, &[&str])] = &[
        ("li", &["list", "installed"]),
        ("la", &["list", "available"]),
        ("ls", &["list", "sync"]),
        ("lu", &["list", "sync"]),
        ("ar", &["repo", "add"]),
        ("lr", &["repo", "list"]),
        ("rr", &["repo", "remove"]),
        ("ur", &["repo", "update"]),
        ("er", &["repo", "enable"]),
        ("dr", &["repo", "disable"]),
        ("ix", &["index"]),
        ("it", &["install"]),
        ("rm", &["remove"]),
        ("up", &["sync"]),
    ];

    let mut args = args.collect::<Vec<_>>();

    for (alias, replacements) in ALIASES {
        let Some(pos) = args.iter().position(|a| a == *alias) else {
            continue;
        };

        args.splice(pos..pos + 1, replacements.iter().map(|&arg| arg.to_owned()));

        break;
    }

    args
}

fn print_system_model_warning(installation: &Installation) {
    eprintln!(
        "{}: `{path:?}` is present & therefore active. This means that:
 
- The system-model is the source of truth and defines all repositories & installed packages.
- Any changes made via `moss` commands will be temporary until the system-model is updated.
- The system state can be reverted to match the system-model state with a `moss sync`.
- To disable the system-model, either remove or rename `{path:?}`.",
        "INFO".green(),
        path = installation.system_model_path(),
    );
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("boot")]
    Boot(#[from] boot::Error),

    #[error("cache")]
    Cache(#[from] cache::Error),

    #[error("index")]
    Index(#[from] index::Error),

    #[error("info")]
    Info(#[from] info::Error),

    #[error("install")]
    Install(#[from] install::Error),

    #[error("list")]
    List(#[from] list::Error),

    #[error("inspect")]
    Inspect(#[from] inspect::Error),

    #[error("extract")]
    Extract(#[from] extract::Error),

    #[error("remove")]
    Remove(#[from] remove::Error),

    #[error("repo")]
    Repo(#[from] repo::Error),

    #[error("search")]
    Search(#[from] search::Error),

    #[error("search-file")]
    SearchFile(#[from] search_file::Error),

    #[error("state")]
    State(#[from] state::Error),

    #[error("sync")]
    Sync(#[from] sync::Error),

    #[error("installation")]
    Installation(#[from] installation::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}
