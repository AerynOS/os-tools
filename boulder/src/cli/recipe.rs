// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::{
    io::{self, Read},
    path::PathBuf,
    time::Duration,
};

use boulder::{
    architecture,
    draft::{self, Drafter},
    macros, recipe, Env, Macros,
};
use clap::Parser;
use fs_err as fs;
use futures_util::StreamExt;
use itertools::Itertools;
use moss::{request, runtime};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tui::{
    pretty::{self, ColumnDisplay},
    MultiProgress, ProgressBar, ProgressStyle, Styled,
};
use url::Url;

#[derive(Debug, Parser)]
#[command(about = "Utilities to create and manipulate stone recipe files")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    #[command(about = "Bump a recipe's release")]
    Bump {
        #[arg(
            short,
            long,
            default_value = "./stone.yaml",
            help = "Location of the recipe file to update"
        )]
        recipe: PathBuf,
        #[arg(
            short = 'n',
            long,
            required = false,
            help = "Set release to a specific number instead of incrementing by one"
        )]
        release: Option<u64>,
    },
    #[command(about = "Create skeletal stone.yaml recipe from source archive URIs")]
    New {
        #[arg(short, long, default_value = ".", help = "Location to output generated files")]
        output: PathBuf,
        #[arg(required = true, value_name = "URI", help = "Source archive URIs")]
        upstreams: Vec<Url>,
    },
    #[command(about = "Update a recipe file")]
    Update {
        #[arg(long = "ver", required = true, help = "Update version")]
        version: String,
        #[arg(
            short = 'u',
            long = "upstream",
            required = true,
            value_parser = parse_upstream,
            help = "Update upstream source, can be passed multiple times. Applied in same order as defined in recipe file.",
            long_help = "Update upstream source, can be passed multiple times. Applied in same order as defined in recipe file.\n\nExample: -u \"https://some.plan/file.tar.gz\" -u \"git|v1.1\"",
        )]
        upstreams: Vec<Upstream>,
        #[arg(help = "Path to recipe file, otherwise read from standard input")]
        recipe: Option<PathBuf>,
        #[arg(
            short = 'w',
            long,
            default_value = "false",
            help = "Overwrite the recipe file in place instead of printing to standard output"
        )]
        overwrite: bool,
        #[arg(long, default_value = "false", help = "Don't increment the release number")]
        no_bump: bool,
    },
    #[command(about = "Print macro definitions")]
    Macros {
        #[arg(name = "macro", help = "Print definition and example for the provided macro")]
        _macro: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum Upstream {
    Plain(Url),
    Git(String),
}

fn parse_upstream(s: &str) -> Result<Upstream, String> {
    match s.strip_prefix("git|") {
        Some(rev) => Ok(Upstream::Git(rev.to_owned())),
        None => Ok(Upstream::Plain(s.parse::<Url>().map_err(|e| e.to_string())?)),
    }
}

pub fn handle(command: Command, env: Env) -> Result<(), Error> {
    match command.subcommand {
        Subcommand::Bump { recipe, release } => bump(recipe, release),
        Subcommand::New { output, upstreams } => new(output, upstreams, env),
        Subcommand::Update {
            recipe,
            overwrite,
            version,
            upstreams,
            no_bump,
        } => update(recipe, overwrite, version, upstreams, no_bump),
        Subcommand::Macros { _macro } => macros(_macro, env),
    }
}

fn bump(recipe: PathBuf, release: Option<u64>) -> Result<(), Error> {
    let path = recipe::resolve_path(&recipe).map_err(Error::ResolvePath)?;
    let input = fs::read_to_string(path).map_err(Error::Read)?;

    // Parsed allows us to access known values in a type safe way
    let parsed: recipe::Parsed = serde_yaml::from_str(&input)?;

    // Bump op
    let prev = parsed.source.release;
    let next = release.unwrap_or(parsed.source.release + 1);
    let mut updater = yaml::Updater::new();
    updater.update_value(next, |root| root / "release");

    // Apply updates
    let updated = updater.apply(input);

    fs::write(&recipe, updated.as_bytes()).map_err(Error::Write)?;
    println!(
        "{}: {} release updated from {prev} to {next}",
        recipe.display(),
        parsed.source.name,
    );

    Ok(())
}

fn new(output: PathBuf, upstreams: Vec<Url>, env: Env) -> Result<(), Error> {
    // We use async to fetch upstreams
    let _guard = runtime::init();

    const RECIPE_FILE: &str = "stone.yaml";
    const MONITORING_FILE: &str = "monitoring.yaml";

    let drafter = Drafter::new(upstreams, env.data_dir);
    let draft = drafter.run()?;

    if !output.is_dir() {
        fs::create_dir_all(&output).map_err(Error::CreateDir)?;
    }

    fs::write(PathBuf::from(&output).join(RECIPE_FILE), draft.stone).map_err(Error::Write)?;
    fs::write(PathBuf::from(&output).join(MONITORING_FILE), draft.monitoring).map_err(Error::Write)?;

    println!("Saved {RECIPE_FILE} & {MONITORING_FILE} to {output:?}");

    Ok(())
}

fn update(
    recipe: Option<PathBuf>,
    overwrite: bool,
    version: String,
    upstreams: Vec<Upstream>,
    no_bump: bool,
) -> Result<(), Error> {
    if overwrite && recipe.is_none() {
        return Err(Error::OverwriteRecipeRequired);
    }

    let input = if let Some(recipe_path) = &recipe {
        let path = recipe::resolve_path(recipe_path).map_err(Error::ResolvePath)?;

        fs::read_to_string(path).map_err(Error::Read)?
    } else {
        let mut bytes = vec![];
        io::stdin().lock().read_to_end(&mut bytes).map_err(Error::Read)?;
        String::from_utf8(bytes)?
    };

    // Parsed allows us to access known values in a type safe way
    let parsed: recipe::Parsed = serde_yaml::from_str(&input)?;
    // Value allows us to access map keys in their original form
    let value: serde_yaml::Value = serde_yaml::from_str(&input)?;

    #[derive(Debug)]
    enum Update {
        Release(u64),
        Version(String),
        PlainUpstream(usize, serde_yaml::Value, Url),
        GitUpstream(usize, serde_yaml::Value, String),
    }

    let mut updates = vec![Update::Version(version)];
    if !no_bump {
        updates.push(Update::Release(parsed.source.release + 1));
    }

    for (i, (original, update)) in parsed.upstreams.iter().zip(upstreams).enumerate() {
        match (original, update) {
            (stone_recipe::Upstream::Plain { .. }, Upstream::Git(_)) => {
                return Err(Error::UpstreamMismatch(i, "Plain", "Git"))
            }
            (stone_recipe::Upstream::Git { .. }, Upstream::Plain(_)) => {
                return Err(Error::UpstreamMismatch(i, "Git", "Plain"))
            }
            (stone_recipe::Upstream::Plain { .. }, Upstream::Plain(new_uri)) => {
                let key = value["upstreams"][i]
                    .as_mapping()
                    .and_then(|map| map.keys().next())
                    .cloned();
                if let Some(key) = key {
                    updates.push(Update::PlainUpstream(i, key, new_uri));
                }
            }
            (stone_recipe::Upstream::Git { .. }, Upstream::Git(new_ref)) => {
                let key = value["upstreams"][i]
                    .as_mapping()
                    .and_then(|map| map.keys().next())
                    .cloned();
                if let Some(key) = key {
                    updates.push(Update::GitUpstream(i, key, new_ref));
                }
            }
        }
    }

    // Needed to fetch
    let _guard = runtime::init();

    let mpb = MultiProgress::new();

    // Add all update operations
    let mut updater = yaml::Updater::new();
    for update in updates {
        match update {
            Update::Release(release) => {
                updater.update_value(release, |root| root / "release");
            }
            Update::Version(version) => {
                updater.update_value(version, |root| root / "version");
            }
            Update::PlainUpstream(i, key, new_uri) => {
                let hash = runtime::block_on(fetch_hash(new_uri.clone(), &mpb))?;

                let path = |root| root / "upstreams" / i / key.as_str().unwrap_or_default();

                // Update hash as either scalar or inner map "hash" value
                updater.update_value(&hash, path);
                updater.update_value(&hash, |root| path(root) / "hash");
                // Update from old to new uri
                updater.update_key(new_uri, path);
            }
            Update::GitUpstream(i, key, new_ref) => {
                let path = |root| root / "upstreams" / i / key.as_str().unwrap_or_default();

                // Update ref as either scalar or inner map "ref" value
                updater.update_value(&new_ref, path);
                updater.update_value(&new_ref, |root| path(root) / "ref");
            }
        }
    }

    let _ = mpb.clear();

    // Apply updates
    let updated = updater.apply(input);

    if overwrite {
        let recipe = recipe.expect("checked above");
        fs::write(&recipe, updated.as_bytes()).map_err(Error::Write)?;
        println!("{} updated", recipe.display());
    } else {
        print!("{updated}");
    }

    Ok(())
}

async fn fetch_hash(uri: Url, mpb: &MultiProgress) -> Result<String, Error> {
    let pb = mpb.add(
        ProgressBar::new(u64::MAX)
            .with_message(format!("{} {}", "Fetching".blue(), uri.to_string().bold()))
            .with_style(
                ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
                    .unwrap()
                    .tick_chars("--=≡■≡=--"),
            ),
    );
    pb.enable_steady_tick(Duration::from_millis(150));

    let mut stream = request::get(uri).await?;

    let mut hasher = Sha256::new();
    // Discard bytes
    let mut out = tokio::io::sink();

    while let Some(chunk) = stream.next().await {
        let bytes = &chunk?;

        pb.inc(bytes.len() as u64);

        hasher.update(bytes);
        out.write_all(bytes).await.map_err(Error::FetchIo)?;
    }

    out.flush().await.map_err(Error::FetchIo)?;

    let hash = hex::encode(hasher.finalize());

    pb.finish();
    mpb.remove(&pb);

    Ok(hash)
}

fn macros(_macro: Option<String>, env: Env) -> Result<(), Error> {
    let macros = Macros::load(&env)?;

    let mut items = macros
        .actions
        .iter()
        .flat_map(|m| {
            m.actions.iter().map(|action| PrintMacro {
                name: format!("%{}", action.key),
                // Multi-line strings need to be in `example`
                description: action.value.description.lines().next().unwrap_or_default(),
                example: action.value.example.as_deref(),
            })
        })
        .sorted()
        .collect::<Vec<_>>();

    let mut definitions = vec![];
    for arch in ["base", &architecture::host().to_string()] {
        if let Some(macros) = macros.arch.get(arch) {
            definitions.extend(macros.definitions.iter().map(|def| PrintMacro {
                name: format!("%({})", def.key),
                description: &def.value,
                example: None,
            }));
        }
    }
    definitions.sort();
    definitions.dedup();

    items.extend(definitions);

    match _macro {
        Some(name) => {
            let Some(action) = items
                .into_iter()
                .find(|a| a.name == format!("%{name}") || a.name == format!("%({name})"))
            else {
                return Err(Error::MacroNotFound(name));
            };

            println!("{} - {}", action.name.bold(), action.description);

            if let Some(example) = action.example {
                println!("\n{}", "Example:".bold());
                for line in example.lines() {
                    println!("  {line}");
                }
            }
        }
        None => {
            pretty::print_columns(&items, 1);
        }
    }

    Ok(())
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct PrintMacro<'a> {
    name: String,
    description: &'a str,
    example: Option<&'a str>,
}

impl ColumnDisplay for PrintMacro<'_> {
    fn get_display_width(&self) -> usize {
        self.name.len()
    }

    fn display_column(&self, writer: &mut impl io::prelude::Write, _col: pretty::Column, width: usize) {
        let _ = write!(
            writer,
            "{}{}  {}",
            self.name.clone().bold(),
            " ".repeat(width),
            self.description,
        );
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Recipe file must be provided to use -w/--overwrite")]
    OverwriteRecipeRequired,
    #[error("Mismatch for upstream[{0}], expected {1} got {2}")]
    UpstreamMismatch(usize, &'static str, &'static str),
    #[error("load macros")]
    LoadMacros(#[from] macros::Error),
    #[error("Macro doesn't exist: {0}")]
    MacroNotFound(String),
    #[error("resolve recipe path")]
    ResolvePath(#[source] recipe::Error),
    #[error("reading recipe")]
    Read(#[source] io::Error),
    #[error("writing recipe")]
    Write(#[source] io::Error),
    #[error("creating output directory")]
    CreateDir(#[source] io::Error),
    #[error("deserializing recipe")]
    Deser(#[from] serde_yaml::Error),
    #[error("fetch upstream")]
    Fetch(#[from] request::Error),
    #[error("fetch upstream")]
    FetchIo(#[source] io::Error),
    #[error("invalid utf-8 input")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("draft")]
    Draft(#[from] draft::Error),
}
