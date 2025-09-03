// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::{collections::HashMap, path::PathBuf};

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
        global = true,
        help = "Move newly built .stone package files to the given repo"
    )]
    pub mv_to_repo: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        requires = "mv-to-repo",
        global = true,
        help = "Auto re-index the repo after a successful build and move"
    )]
    pub re_index: bool,
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

    match subcommand {
        Some(Subcommand::Build(_)) | Some(Subcommand::Recipe(_)) => { /* do nothing, the flags were passed in appropriately. */
        }
        _ => match (global.mv_to_repo.clone(), global.re_index) {
            (Some(_), false) => {
                eprintln!(
                    "{}: The `--mv-to-repo` flag cannot be used with anything but the `build` or `recipe` subcommands",
                    "Error".red()
                );
                std::process::exit(1);
            }
            (None, true) => {
                eprintln!(
                    "{}: The `--re-index` cannot be used with anything but the `build` or `recipe` subcommands and requires `--mv-to-repo`",
                    "Error".red()
                );
                std::process::exit(1);
            }
            (Some(_), true) => {
                eprintln!(
                    "{}: The ``--mv-to-repo` and ``--re-index` flags can only be used with the `build` or `recipe` subcommands",
                    "Error".red()
                );
                std::process::exit(1);
            }
            (None, false) => { /* do nothing, the flags weren't passed in. */ }
        },
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
                        // Check to see if the repo is in moss
                        let moss_cmd = std::process::Command::new("moss")
                            .args(["repo", "list"])
                            .stdout(std::process::Stdio::piped())
                            .output()
                            .expect("Couldn't get a list of moss repos");

                        // Convert the output to a String
                        let repos = String::from_utf8(moss_cmd.stdout).expect("Could get the repo list from moss");

                        let mv_repo = repos
                            .lines()
                            .filter_map(|line| {
                                if line.contains(&repo) {
                                    let mut ret_map = HashMap::new();
                                    println!("{}: {line}", "DEBUG".yellow());
                                    let uri = line
                                        .split_whitespace()
                                        .filter(|line| line.contains("//"))
                                        .last()
                                        .and_then(|uri| {
                                            if uri.contains("file:///") {
                                                Some(uri.to_string().replace("file://", ""))
                                            } else {
                                                Some(uri.to_string())
                                            }
                                        })
                                        .expect("Couldn't get URI from repo string".red().to_string().as_str());

                                    let _ = ret_map.insert(&repo, Some(uri.clone()));

                                    Some(ret_map)
                                } else {
                                    None
                                }
                            })
                            .last()
                            .unwrap_or_else(|| HashMap::new());

                        // Check to ensure that the repo has a URI;
                        // return Err if there isn't.
                        if mv_repo.get(&repo).is_none() {
                            eprintln!("{} {}", &repo, "is not a valid repo registered with moss");
                            return Err(Error::Build(build::Error::Build(boulder::build::Error::InvalidRepo)));
                        }

                        // Move the newly built .stone files
                        match mv_to_repo(&repo, &mv_repo) {
                            Ok(repo) => {
                                if global.re_index && repo.is_some() {
                                    if let Err(err) =
                                        re_index_repo(&repo.expect(
                                            format!("{}: Repo was supposed to be Some", "Error".red()).as_str(),
                                        ))
                                    {
                                        eprintln!("{} {}", "Error:".red(), err.to_string().red());
                                        return Err(err);
                                    }
                                } else if global.re_index && repo.is_none() {
                                    eprintln!("{}", "Error: Cannot re-index, returned repo name was empty!".red());
                                    return Err(Error::Reindex(
                                        "Cannot re-index, move operation returned an invalid repo name"
                                            .red()
                                            .to_string(),
                                    ));
                                }
                            }
                            Err(err) => {
                                eprintln!("{} {}", "Error:".red(), err.to_string().red());
                                return Err(err);
                            }
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
            if let Some(repo) = global.mv_to_repo {
                // Check to see if the repo is in moss
                let moss_cmd = std::process::Command::new("moss")
                    .args(["repo", "list"])
                    .output()
                    .expect("Couldn't get a list of moss repos");

                let repos = String::from_utf8(moss_cmd.stdout).expect("Could get the repo list from moss");

                let mv_repo = repos
                    .lines()
                    .filter_map(|line| {
                        if line.contains(&repo) {
                            let mut ret_map = HashMap::new();

                            let uri = line
                                .split_whitespace()
                                .filter(|line| line.contains("//"))
                                .last()
                                .and_then(|uri| {
                                    if uri.contains("file:///") {
                                        Some(uri.to_string().replace("file://", ""))
                                    } else {
                                        Some(uri.to_string())
                                    }
                                })
                                .expect("Couldn't get URI from repo string".red().to_string().as_str());

                            let _ = ret_map.insert(&repo, Some(uri.clone()));

                            Some(ret_map)
                        } else {
                            None
                        }
                    })
                    .last()
                    .unwrap_or_else(|| HashMap::new());

                if mv_repo.get(&repo).is_none() {
                    eprintln!("{} {}", &repo, "is not a valid repo registered with moss");
                    return Err(Error::Build(build::Error::Build(boulder::build::Error::InvalidRepo)));
                }

                recipe::handle(command, env)?;

                match mv_to_repo(&repo, &mv_repo) {
                    Ok(repo) => {
                        // Ok to re-index as there is a value use
                        if global.re_index && repo.is_some() {
                            if let Err(err) = re_index_repo(&repo.clone().expect(
                                format!("{}: Returned repo should've been Some", "Error".red().to_string()).as_str(),
                            )) {
                                eprintln!("{} {}", "Error:".red(), err.to_string().red());
                                return Err(Error::Reindex(
                                    "Cannot re-index, move operation returned an invalid repo name"
                                        .red()
                                        .to_string(),
                                ));
                            }
                        } else if global.re_index && repo.is_none() {
                            eprintln!("{}", "Error: Cannot re-index, returned repo name was empty!".red());
                            return Err(Error::Reindex(
                                "Cannot re-index, move operation returned an invalid repo name"
                                    .red()
                                    .to_string(),
                            ));
                        }
                    }
                    Err(err) => {
                        eprintln!("{} {}", "Error:".red(), err.to_string().red());
                        return Err(err);
                    }
                }
            } else {
                recipe::handle(command, env)?;
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

fn mv_to_repo(repo_key: &String, repo_map: &HashMap<&String, Option<String>>) -> Result<Option<String>, Error> {
    let repo_path = repo_map.get(repo_key).unwrap_or_else(|| &None);
    if let Some(repo_path) = repo_path {
        let cwd = PathBuf::from(".");
        let manifest_ext = "stone";

        let repo_path = PathBuf::from(if repo_path.contains("file://") {
            repo_path.replacen("file://", "", 1).replacen("stone.index", "", 1)
        } else {
            repo_path.to_string().replacen("stone.index", "", 1)
        });

        println!("{}: {repo_path:?}", "Debug".yellow());

        // Create repo directory if it doesn't exist
        if !repo_path.exists() {
            fs::create_dir_all(&repo_path).expect("Failed to create {repo_key} repo directories");
        }

        match fs::read_dir(&cwd) {
            Ok(dir) => {
                for (_, pkg_file) in dir.enumerate() {
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

                            println!(
                                "{} {} {} {}\n",
                                "Moving".blue(),
                                &path.to_string_lossy().to_string().blue().italic().bold(),
                                "to".blue(),
                                &dest_path.to_string_lossy().to_string().blue().italic().bold()
                            );
                            match fs::rename(&path, &dest_path) {
                                Ok(_) => {
                                    println!(
                                        "{} {} {} {}\n",
                                        "Successfully moved".green(),
                                        &path.to_string_lossy().to_string().green().italic().bold(),
                                        "to".green(),
                                        &dest_path.to_string_lossy().to_string().green().italic().bold()
                                    );
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
                                }
                            }
                        }
                    }
                }

                return Ok(Some(repo_path.to_string_lossy().to_string()));
            }
            Err(e) => {
                eprintln!("Failed to read directory: {e}");
                return Err(Error::Io(e));
            }
        }
    }

    if let Some(repo_path) = repo_path {
        Ok(Some(repo_path.clone()))
    } else {
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{}: {repo_key} doesn't have a valid path", "Error".red()).as_str(),
        )))
    }
}

fn re_index_repo(repo: &str) -> Result<(), Error> {
    use std::process::{Command as Cmd, Stdio};

    let mut moss_cmd = Cmd::new("moss")
        .args(["index", repo])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    let _index_status = moss_cmd.wait()?;

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
    #[error("reindex")]
    Reindex(String),
}
