// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::path::PathBuf;

use boulder::{Env, env};
use clap::{Args, CommandFactory, Parser};
use clap_complete::{
    generate_to,
    shells::{Bash, Fish, Zsh},
};
use clap_mangen::Man;
use fs_err::{self as fs, File};
use thiserror::Error;
use tui::Styled;

mod build;
mod chroot;
mod profile;
mod recipe;
mod version;

#[derive(Debug, Parser)]
pub struct Command {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,
}

#[derive(Debug, Args)]
pub struct Global {
    #[arg(
        short,
        long = "verbose",
        help = "Prints additional information about what boulder is doing",
        default_value = "false",
        global = true
    )]
    pub verbose: bool,
    #[arg(short = 'V', long, default_value = "false", global = true)]
    pub version: bool,
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub moss_root: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    pub generate_manpages: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    pub generate_completions: Option<PathBuf>,
    #[arg(
        long,
        require_equals = true,
        value_parser = clap::builder::PossibleValuesParser::new(["local", "unstable", "volatile"]
        ),
        help = "Move newly built .stone package files to the given repo"
    )]
    pub mv_to_repo: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    Build(build::Command),
    Chroot(chroot::Command),
    Profile(profile::Command),
    Recipe(recipe::Command),
    Version(version::Command),
}

pub fn process() -> Result<(), Error> {
    let args = replace_aliases(std::env::args());
    let Command { global, subcommand } = Command::parse_from(args.clone());

    // Prints the cli's information about version at startup
    if global.version {
        println!("boulder {}", tools_buildinfo::get_full_version());
    }

    if let Some(dir) = global.generate_manpages {
        fs::create_dir_all(&dir)?;
        let main_cmd = Command::command();
        // Generate man page for the main command
        let main_man = Man::new(main_cmd.clone());
        let mut buffer = File::create(dir.join("boulder.1"))?;
        main_man.render(&mut buffer)?;

        // Generate man pages for all subcommands
        for sub in main_cmd.get_subcommands() {
            let sub_man = Man::new(sub.clone());
            let name = format!("boulder-{}.1", sub.get_name());
            let mut buffer = File::create(dir.join(&name))?;
            sub_man.render(&mut buffer)?;

            // Generate man pages for nested subcommands
            for nested in sub.get_subcommands() {
                let nested_man = Man::new(nested.clone());
                let name = format!("boulder-{}-{}.1", sub.get_name(), nested.get_name());
                let mut buffer = File::create(dir.join(&name))?;
                nested_man.render(&mut buffer)?;
            }
        }
        return Ok(());
    }

    if let Some(dir) = global.generate_completions {
        fs::create_dir_all(&dir)?;
        let mut cmd = Command::command();
        generate_to(Bash, &mut cmd, "boulder", &dir)?;
        generate_to(Fish, &mut cmd, "boulder", &dir)?;
        generate_to(Zsh, &mut cmd, "boulder", &dir)?;
        return Ok(());
    }

    if global.mv_to_repo.is_some() {
        match subcommand {
            Some(Subcommand::Build(_)) | Some(Subcommand::Recipe(_)) => {}
            _ => {
                eprintln!(
                    "{}",
                    "The `--mv-to-repo` flag must be used with either the build or recipe subcommand"
                        .red()
                        .to_string()
                );
                std::process::exit(1);
            }
        }
    }

    let env = Env::new(global.cache_dir, global.config_dir, global.data_dir, global.moss_root)?;

    if global.verbose {
        match subcommand {
            Some(Subcommand::Version(_)) => (),
            _ => version::print(),
        }
        println!("{:?}", env.config);
        println!("cache directory: {:?}", env.cache_dir);
        println!("data directory: {:?}", env.data_dir);
        println!("moss directory: {:?}", env.moss_dir);
    }

    match subcommand {
        Some(Subcommand::Build(command)) => {
            match build::handle(command, env) {
                Ok(_) => {
                    if let Some(repo) = global.mv_to_repo {
                        if let Err(err) = mv_to_repo(&repo) {
                            eprintln!("{} {}", "Error:".red(), err.to_string().red());
                            return Err(err);
                        }
                    }
                }
                Err(e) => {
                    let err_str = e.to_string().red();
                    eprintln!("{err_str}");
                    return Err(Error::Build(e));
                }
            };
        }
        Some(Subcommand::Chroot(command)) => chroot::handle(command, env)?,
        Some(Subcommand::Profile(command)) => profile::handle(command, env)?,
        // Recipe takes into account the global.build flag
        Some(Subcommand::Recipe(command)) => {
            // Give an error message and exit without running the command
            // if the --mv-to-repo flag was give without the --build flag.
            if global.mv_to_repo.is_some() && !command.build {
                eprintln!("Error: Cannot use `--mv-to-repo` without the `--build` flag");
                std::process::exit(1);
            }
            recipe::handle(command, env)?;

            if let Some(repo) = global.mv_to_repo {
                if let Err(err) = mv_to_repo(&repo) {
                    eprintln!("{} {}", "Error:".red(), err.to_string().red());
                    return Err(err);
                }
            }
        }
        Some(Subcommand::Version(command)) => version::handle(command),
        None => (),
    }

    Ok(())
}

fn replace_aliases(args: std::env::Args) -> Vec<String> {
    const ALIASES: &[(&str, &[&str])] = &[
        ("bump", &["recipe", "bump"]),
        ("new", &["recipe", "new"]),
        ("macros", &["recipe", "macros"]),
        ("up", &["recipe", "update"]),
    ];

    let mut args = args.collect::<Vec<_>>();

    for (alias, replacements) in ALIASES {
        let Some(pos) = args.iter().position(|a| a == *alias) else {
            continue;
        };

        // Escape hatch for alias w/ same name as
        // inner subcommand
        if args.get(pos - 1).map(String::as_str) == replacements.first().copied() {
            continue;
        }

        args.splice(pos..pos + 1, replacements.iter().map(|&arg| arg.to_owned()));

        break;
    }

    args
}

fn mv_to_repo(repo: &String) -> Result<(), Error> {
    let repo_path = if repo == "local" {
        dirs::home_dir()
            .expect(&format!("{}", "Failed to get home directory".red()))
            .join(".cache/local_repo/x86_64")
            .to_string_lossy()
            .to_string()
    } else if repo == "volatile" {
        println!("TODO: Move to volatile repo");
        String::new()
    } else {
        println!("TODO: Move to unstable repo");
        String::new()
    };

    if !repo_path.is_empty() {
        let cwd = PathBuf::from(".");
        let manifest_ext = "stone";

        // Create repo directory if it doesn't exist
        fs::create_dir_all(&repo_path).expect("Failed to create local repo directories");
        match fs::read_dir(&cwd) {
            Ok(dir) => {
                for pkg_file in dir {
                    let pkg_file = pkg_file.expect("Failed to get package file to move");
                    let path = pkg_file.path();

                    if path.is_file()
                        && let Some(ext) = path.extension().and_then(|ext| ext.to_str())
                    {
                        if ext == manifest_ext {
                            let file_name = path
                                .file_name()
                                .ok_or("Invalid package file name")
                                .expect("Failed to get package file name");
                            let dest_path = PathBuf::from(&repo_path).join(file_name);

                            println!("Moving {:?} to {:?}", &path, &dest_path);
                            match fs::rename(&path, &dest_path) {
                                Ok(_) => {
                                    println!("Successfully moved {:?} to {:?}", &path, &dest_path);
                                    return Ok(());
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{}",
                                        &format!(
                                            "{} {} {} {} {} {}",
                                            "Failed to move".red(),
                                            &path.to_string_lossy().to_string().red(),
                                            "to".red(),
                                            &dest_path.to_string_lossy().to_string().red(),
                                            ":".red(),
                                            e.to_string().red()
                                        )
                                    );
                                    return Err(Error::Io(e));
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read directory: {e}");
                return Err(Error::Io(e));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("build")]
    Build(#[from] build::Error),
    #[error("chroot")]
    Chroot(#[from] chroot::Error),
    #[error("profile")]
    Profile(#[from] profile::Error),
    #[error("env")]
    Env(#[from] env::Error),
    #[error("recipe")]
    Recipe(#[from] recipe::Error),
    #[error("io error")]
    Io(#[from] std::io::Error),
}
