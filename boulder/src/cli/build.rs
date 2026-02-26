// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::io;
use std::num::NonZeroU64;
use std::path::PathBuf;

use crate::build::{self, Builder};
use crate::package::Packager;
use crate::{Env, Timing, container, package, profile, timing};
use chrono::Local;
use clap::Parser;
use moss::signal::inhibit;
use thiserror::Error;
use thread_priority::{NormalThreadSchedulePolicy, ThreadPriority, ThreadSchedulePolicy, thread_native_id};
use tui::Styled;
use version_parse::VersionExtractor;

#[derive(Debug, Parser)]
#[command(about = "Build stone package(s) from a stone recipe file")]
pub struct Command {
    #[arg(short, long, default_value = "default-x86_64")]
    profile: profile::Id,
    #[arg(
        short,
        long = "compiler-cache",
        help = "Enable compiler caching",
        default_value_t = false
    )]
    ccache: bool,
    #[arg(
        short,
        long,
        default_value_t = false,
        help = "Update profile repositories before building"
    )]
    update: bool,
    #[arg(
        long = "normal-priority",
        help = "Run the build without lowering the process priority",
        default_value_t = false
    )]
    normal_priority: bool,
    #[arg(short, long, default_value = ".", help = "Directory to store build results")]
    output: PathBuf,
    #[arg(default_value = "./stone.yaml", help = "Path to recipe file")]
    recipe: PathBuf,
    #[arg(
        short,
        long,
        default_value = "1",
        help = "Specify the build release number used for this build"
    )]
    build_release: NonZeroU64,
    #[arg(
        long,
        help = "Automatically cleanup all build related artefacts",
        default_value_t = false
    )]
    cleanup: bool,
    /// Verify the built manifest against the provided [MANIFEST] file and fail the build if they don't match
    ///
    /// If supplied & the manifests do match, the existing manifests are preserved instead of being overwritten
    #[arg(long = "verify", value_name = "MANIFEST")]
    verify_against: Option<PathBuf>,
}

pub fn handle(command: Command, env: Env) -> Result<(), Error> {
    let output = command.output.clone();
    let Command {
        profile,
        recipe: recipe_path,
        ccache,
        update,
        normal_priority,
        build_release,
        cleanup,
        verify_against,
        ..
    } = command;

    let mut timing = Timing::default();
    let timer = timing.begin(timing::Kind::Initialize);

    if !output.exists() {
        return Err(Error::MissingOutput(output));
    }

    // Ensure verify against path isn't json/jsonc since
    // we verify against binary manifest
    if let Some(path) = verify_against.as_ref()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.starts_with("json"))
    {
        return Err(Error::VerifyBinaryManifestRequired(path.to_owned()));
    }

    let builder = Builder::new(&recipe_path, verify_against.clone(), env, profile, ccache, output)?;
    let pkg_name = format!(
        "{}-{}-{}",
        builder.recipe.parsed.source.name, builder.recipe.parsed.source.version, builder.recipe.parsed.source.release
    );
    println!("boulder {}", tools_buildinfo::get_simple_version());
    println!("└─ building {pkg_name}-{build_release}\n");
    builder.setup(&mut timing, timer, update)?;

    let paths = &builder.paths;
    let networking = builder.recipe.parsed.options.networking;

    // Set the current thread priority to SCHED_BATCH so that it's inherited by all child processes
    if !normal_priority {
        thread_priority::set_thread_priority_and_policy(
            thread_native_id(),
            ThreadPriority::Min,
            ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Batch),
        )?;
    }

    // hold a fd
    let _fd = inhibit(
        vec!["shutdown", "sleep", "idle", "handle-lid-switch"],
        "boulder".into(),
        format!("Build in-progress: {pkg_name}"),
        "block".into(),
    );

    // Build & package from within container
    container::exec::<Error>(paths, networking, || {
        builder.build(&mut timing)?;

        let packager = Packager::new(
            &builder.paths,
            &builder.recipe,
            &builder.macros,
            &builder.targets,
            build_release,
        )?;
        packager.package(&mut timing)?;

        timing.print_table();

        Ok(())
    })?;

    // Copy artefacts to host recipe dir
    package::sync_artefacts(paths).map_err(Error::SyncArtefacts)?;

    if cleanup {
        builder.cleanup().map_err(Error::Cleanup)?;
    }

    verify_versions_match(&builder)?;

    println!(
        "Build finished successfully at {}",
        Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );

    Ok(())
}

fn verify_versions_match(builder: &Builder) -> Result<(), Error> {
    // Only check the first upstream, as that'll be the actual version
    // in the majority of cases.
    let first_upstream = if let Some(get) = builder.recipe.parsed.upstreams.first() {
        get
    } else {
        // We may not have any upstreams for meta packages
        return Ok(());
    };

    // We won't attempt to parse git upstreams for now
    match &first_upstream.props {
        stone_recipe::upstream::Props::Git { git_ref, staging: _ } => {
            // If we have a git ref, we have a git upstream and version parsing
            // will not work.
            if !git_ref.is_empty() {
                return Ok(());
            }
        }
        stone_recipe::upstream::Props::Plain {
            hash: _,
            rename: _,
            strip_dirs: _,
            unpack: _,
        } => {}
    }

    let ver_ext = VersionExtractor::new();
    let parsed_upstream = ver_ext.extract(first_upstream.url.as_str());
    match parsed_upstream {
        Ok(ext) => {
            if ext.version != builder.recipe.parsed.source.version {
                println!(
                    "{} | 'version' and first parsed upstream version do not match. Expected: {}, got: {}",
                    "Warning".yellow(),
                    ext.version,
                    builder.recipe.parsed.source.version
                );
                println!();
            }
        }
        // Swallow error if the url couldn't be parsed
        Err(_) => return Ok(()),
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("output directory does not exist: {0:?}")]
    MissingOutput(PathBuf),
    #[error("build recipe")]
    Build(#[from] build::Error),
    #[error("package artifacts")]
    Package(#[from] package::Error),
    #[error("sync artefacts")]
    SyncArtefacts(#[source] io::Error),
    #[error("container")]
    Container(#[from] container::Error),
    #[error("setting thread priority")]
    Priority(#[from] thread_priority::Error),
    #[error("cleanup")]
    Cleanup(#[source] build::Error),
    #[error("Binary manifest required for verification, got {0:?}")]
    VerifyBinaryManifestRequired(PathBuf),
    #[error("version parse")]
    Upstreams(#[from] version_parse::VersionError),
}
