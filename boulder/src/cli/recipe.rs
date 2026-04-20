// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0
use std::{
    io::{self, Read},
    path::PathBuf,
    time::Duration,
};

use crate::{
    Env, Macros, architecture,
    draft::{self, Drafter, upstream::fetched_upstream_cache_path},
    macros, recipe,
};
use clap::Parser;
use ent_core::{data::updates::get_latest_version, recipes::ParserRegistration};
use fs_err::{self as fs};
use itertools::Itertools;
use moss::{request, runtime, util};
use similar::TextDiff;
use stone_recipe::upstream;
use tempfile::NamedTempFile;
use thiserror::Error;
use tui::{
    MultiProgress, ProgressBar, ProgressStyle, Styled,
    dialoguer::{Confirm, theme::ColorfulTheme},
    pretty::{self, ColumnDisplay},
};
use url::Url;
use version_parse::VersionExtractor;

const LONG_UPDATE_ABOUT: &str = concat!(
    "Update a recipe file\n\n",
    "If no version or upstreams are provided then the recipe will attempt to be autoupdated\n",
    "using the release information supplied in the monitoring.yaml file.\n\n",
    "If a version is passed but no upstream, boulder will attempt to guess the new url from\n",
    "the existing url.\n\n",
    "If an upstream is passed but no version is passed, boulder will parse the new version\n",
    "from the new upstream."
);

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
    #[command(about = LONG_UPDATE_ABOUT)]
    Update {
        #[arg(id = "recipe_version", long = "ver", required = false, help = "Update version")]
        version: Option<String>,
        #[arg(
            short = 'u',
            long = "upstream",
            required = false,
            value_parser = parse_updated_source,
            help = concat!(
                "Update upstream source, can be passed multiple times.\n",
                "Applied in same order as defined in recipe file. To update a Git upstream,\n",
                "Use the \"git|commit_or_tag\" syntax.\n\n",
                "Example: -u \"https://some.plan/file.tar.gz\" -u \"git|v1.1\"")
        )]
        upstreams: Vec<UpdatedSource>,
        #[arg(
            default_value = "./stone.yaml",
            help = "Path to recipe file, use '-' to read from standard input"
        )]
        recipe: PathBuf,
        #[arg(
            short = 'w',
            long = "write",
            alias = "overwrite",
            help = "Path to write the updated recipe to. Use '-' for standard output. If omitted, defaults to the recipe path."
        )]
        write: Option<PathBuf>,
        #[arg(long, default_value = "false", help = "Don't increment the release number")]
        no_bump: bool,
    },
    #[command(about = "Print macro definitions")]
    Macros {
        #[arg(name = "macro", help = "Print definition and example for the provided macro")]
        _macro: Option<String>,
    },
}

/// A new source for an existing recipe.
#[derive(Clone, Debug)]
pub enum UpdatedSource {
    /// The new source is a regular URL that points
    /// to a source archive.
    Plain(Url),
    /// The new source is a Git reference (i.e. commit hash
    /// or tag) in the Git repository referenced in the recipe.
    Git(String),
}

fn parse_updated_source(s: &str) -> Result<UpdatedSource, String> {
    match s.strip_prefix(upstream::GIT_PREFIX) {
        Some(git_ref) => Ok(UpdatedSource::Git(git_ref.to_owned())),
        None => Ok(UpdatedSource::Plain(s.parse::<Url>().map_err(|e| e.to_string())?)),
    }
}

pub fn handle(command: Command, env: Env, yes: bool) -> Result<(), Error> {
    match command.subcommand {
        Subcommand::Bump { recipe, release } => bump(recipe, release),
        Subcommand::New { output, upstreams } => new(env, output, upstreams),
        Subcommand::Update {
            recipe,
            write,
            version,
            upstreams,
            no_bump,
        } => update(env, recipe, write, version, upstreams, no_bump, yes),
        Subcommand::Macros { _macro } => macros(_macro, env),
    }
}

fn autoupdate(env: Env, recipe: PathBuf, yes: bool) -> Result<(), Error> {
    // TODO: We neednessly reparse here when coming from update()
    //       but, we need the path regardless to parse with ent.
    let path = recipe::resolve_path(&recipe).map_err(Error::ResolvePath)?;
    let input = fs::read_to_string(path).map_err(Error::Read)?;

    let parsed_recipe: recipe::Parsed = serde_yaml::from_str(&input)?;

    // Setup ent parser
    // TODO: Can we avoid the inventory dep and parse the stone directly?
    let registration = inventory::iter::<ParserRegistration>
        .into_iter()
        .find(|p| p.name == "stone_recipe")
        .expect("Stone parser registration missing");
    let ent_parser = (registration.parser)();

    // Parse our recipe with ent
    let ent_parsed = ent_parser.parse(recipe.as_path())?;

    if let Some(m) = ent_parsed.monitoring {
        // Call the release-monitoring.org API using the ID found in monitoring.yaml
        let response = runtime::block_on(get_latest_version(m.project_id))?;

        let current_version = parsed_recipe.source.version;

        let newest = response
            .stable_versions
            .first()
            .cloned()
            .unwrap_or_else(|| response.latest_version.unwrap_or_default());

        println!("Newest version found: {newest}, current version: {current_version}");

        if newest == current_version {
            println!("Already up-to-date!");
            return Ok(());
        }

        // Only parse the first upstream source for now...
        let (first_upstream, _) = parsed_recipe
            .upstreams
            .split_first()
            .expect("upstreams must not be empty");

        let new_url = guess_new_url(newest.as_str(), first_upstream.url.as_str())?;

        let updated_source = parse_updated_source(new_url.as_str()).unwrap();

        update(
            env,
            recipe.clone(),
            None,
            Some(newest),
            vec![updated_source],
            false,
            yes,
        )?;
    };

    Ok(())
}

fn guess_new_url(new_version: &str, current_url: &str) -> Result<String, Error> {
    let upstreams_parser = VersionExtractor::new();
    let parsed_upstream = upstreams_parser.extract(current_url)?;
    println!(
        "Parsed URI: name = {}, version = {}, series-version = {:?}",
        parsed_upstream.name, parsed_upstream.version, parsed_upstream.series_version
    );

    let current_version = &parsed_upstream.version;

    let new_series_version = parsed_upstream
        .series_version
        .as_deref()
        .map(|sv| (sv, derive_series_version(sv, new_version)));

    Ok(current_url
        .split('/')
        .map(|segment| {
            if let Some((old_sv, ref new_sv)) = new_series_version {
                let segment_stripped = segment.trim_start_matches('v');
                if segment_stripped == old_sv {
                    // Preserve the v prefix if the segment had one
                    return if segment.starts_with('v') {
                        format!("v{new_sv}")
                    } else {
                        new_sv.clone()
                    };
                }
            }
            if segment.contains(current_version.as_str()) {
                segment.replace(current_version.as_str(), new_version)
            } else {
                segment.to_owned()
            }
        })
        .join("/"))
}

fn derive_series_version(old_series_version: &str, new_version: &str) -> String {
    let segment_count = old_series_version.split('.').count();

    new_version.split('.').take(segment_count).join(".")
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

fn new(env: Env, output: PathBuf, upstreams: Vec<Url>) -> Result<(), Error> {
    const RECIPE_FILE: &str = "stone.yaml";
    const MONITORING_FILE: &str = "monitoring.yaml";

    let drafter = Drafter::new(env, upstreams);
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
    env: Env,
    recipe: PathBuf,
    write: Option<PathBuf>,
    version: Option<String>,
    sources: Vec<UpdatedSource>,
    no_bump: bool,
    yes: bool,
) -> Result<(), Error> {
    let is_stdin = *recipe == *"-";

    let output_path = match write {
        Some(p) => {
            if p == *"-" {
                None
            } else {
                Some(p)
            }
        }
        None => {
            if is_stdin {
                None
            } else {
                Some(recipe.clone())
            }
        }
    };

    if output_path.is_none() && (version.is_none() && sources.is_empty()) && !is_stdin {
        return Err(Error::OverwriteNotEnabled);
    }

    let input = if !is_stdin {
        let path = recipe::resolve_path(&recipe).map_err(Error::ResolvePath)?;

        if version.is_none() && sources.is_empty() {
            return autoupdate(env, path, yes);
        }

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

    // If version isn't specified guess it from parsing the first upstream url
    let version = if let Some(v) = version {
        v
    } else {
        let (first_upstream, _) = sources.split_first().expect("sources must not be empty");
        let ver_ext = VersionExtractor::new();
        match first_upstream {
            UpdatedSource::Git(_) => return Err(Error::GitUpstreamMustProvideVersion),
            UpdatedSource::Plain(new_uri) => {
                let parsed_upstream = ver_ext.extract(new_uri.as_str())?;
                println!("No version provided, guessed: {}", parsed_upstream.version);
                parsed_upstream.version
            }
        }
    };

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

    for (i, (original, update)) in parsed.upstreams.into_iter().zip(sources).enumerate() {
        match (original.props, update) {
            (upstream::Props::Plain { .. }, UpdatedSource::Git(_)) => {
                return Err(Error::UpstreamMismatch(i, "Plain", "Git"));
            }
            (upstream::Props::Git { .. }, UpdatedSource::Plain(_)) => {
                return Err(Error::UpstreamMismatch(i, "Git", "Plain"));
            }
            (upstream::Props::Plain { .. }, UpdatedSource::Plain(new_uri)) => {
                let key = value["upstreams"][i]
                    .as_mapping()
                    .and_then(|map| map.keys().next())
                    .cloned();
                if let Some(key) = key {
                    updates.push(Update::PlainUpstream(i, key, new_uri));
                }
            }
            (upstream::Props::Git { .. }, UpdatedSource::Git(new_ref)) => {
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

    let mpb = MultiProgress::new();

    // Add all update operations
    let mut updater = yaml::Updater::new();
    for update in updates {
        match update {
            Update::Release(release) => {
                updater.update_value(release, |root| root / "release");
            }
            Update::Version(version) => {
                updater.update_value(format!("\"{version}\""), |root| root / "version");
            }
            Update::PlainUpstream(i, key, new_uri) => {
                let hash = runtime::block_on(fetch_and_cache_upstream(&env, new_uri.clone(), &mpb))?;

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
    let updated = updater.apply(input.clone());

    if let Some(path) = output_path {
        if !yes {
            let diff = TextDiff::from_lines(&input, &updated);
            println!("{}", diff.unified_diff());
            let write_updated_recipe = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(" Do you wish to continue? ")
                .default(false)
                .interact()?;
            if !write_updated_recipe {
                return Ok(());
            }
        }
        fs::write(&path, updated.as_bytes()).map_err(Error::Write)?;
        println!("{} updated", path.display());
    } else {
        print!("{updated}");
    }

    Ok(())
}

/// Fetches the upstream at `uri` and caches it so it doesn't need to be refetched
/// when this recipe is finally built.
///
/// Returns the sha256 hash of the fetched upstream
async fn fetch_and_cache_upstream(env: &Env, uri: Url, mpb: &MultiProgress) -> Result<String, Error> {
    use fs_err::tokio::{self as fs};

    let pb = mpb.add(
        ProgressBar::new(u64::MAX)
            .with_message(format!("{} {}", "Fetching".blue(), uri.as_str().bold()))
            .with_style(
                ProgressStyle::with_template(" {spinner} {wide_msg} {binary_bytes_per_sec:>.dim} ")
                    .unwrap()
                    .tick_chars("--=≡■≡=--"),
            ),
    );
    pb.enable_steady_tick(Duration::from_millis(150));

    let temp_file_path = NamedTempFile::with_prefix("boulder-")
        .map_err(Error::CreateTempFile)?
        .into_temp_path();

    let hash = request::download_with_progress_and_sha256(uri.clone(), &temp_file_path, |progress| {
        pb.inc(progress.delta);
    })
    .await?;

    // Move fetched asset to cache dir so we don't need to refetch it
    // when the user finally builds this new recipe
    {
        let cache_path = fetched_upstream_cache_path(env, &uri, &hash);

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).await.map_err(Error::CreateDir)?;
        }

        util::async_hardlink_or_copy(&temp_file_path, &cache_path)
            .await
            .map_err(Error::MoveTempFile)?;

        drop(temp_file_path);
    }

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
    #[error("Overwrite must be enabled if auto-updating recipe")]
    OverwriteNotEnabled,
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
    #[error("create temp file")]
    CreateTempFile(#[source] io::Error),
    #[error("move temp file")]
    MoveTempFile(#[source] io::Error),
    #[error("fetch upstream")]
    Fetch(#[from] request::Error),
    #[error("invalid utf-8 input")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("draft")]
    Draft(#[from] draft::Error),
    #[error("statuscode")]
    StatusCode(#[from] reqwest::Error),
    #[error("version parse")]
    Upstreams(#[from] version_parse::VersionError),
    #[error("Must provide version if first upstream provided is of type git")]
    GitUpstreamMustProvideVersion,
    #[error("ent recipe parse failure")]
    Ent(#[from] ent_core::recipes::RecipeError),
    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),
    #[error("io")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guess_new_url() {
        let new_url = guess_new_url(
            "1.52.7",
            "https://download.gnome.org/sources/NetworkManager/1.50/NetworkManager-1.50.0.tar.xz",
        )
        .unwrap();
        assert_eq!(
            new_url,
            "https://download.gnome.org/sources/NetworkManager/1.52/NetworkManager-1.52.7.tar.xz"
        );

        let new_url = guess_new_url("9.0.1", "https://www.nano-editor.org/dist/v8/nano-8.7.1.tar.xz").unwrap();
        assert_eq!(new_url, "https://www.nano-editor.org/dist/v9/nano-9.0.1.tar.xz");

        let new_url = guess_new_url("50.0", "https://download.gnome.org/sources/ghex/48/ghex-48.3.tar.xz").unwrap();
        assert_eq!(new_url, "https://download.gnome.org/sources/ghex/50/ghex-50.0.tar.xz");

        let new_url = guess_new_url(
            "1.91.2",
            "https://gitlab.freedesktop.org/upower/upower/-/archive/v1.90.10/upower-v1.90.10.tar.gz",
        )
        .unwrap();
        assert_eq!(
            new_url,
            "https://gitlab.freedesktop.org/upower/upower/-/archive/v1.91.2/upower-v1.91.2.tar.gz"
        );

        let new_url = guess_new_url(
            "260.1",
            "https://github.com/systemd/systemd/archive/refs/tags/v257.13.tar.gz",
        )
        .unwrap();
        assert_eq!(
            new_url,
            "https://github.com/systemd/systemd/archive/refs/tags/v260.1.tar.gz"
        );
    }
}
