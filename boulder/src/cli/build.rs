// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io;
use std::num::NonZeroU64;
use std::path::PathBuf;

use boulder::build::{self, Builder};
use boulder::package::Packager;
use boulder::{Env, Timing, container, package, profile, timing};
use chrono::Local;
use clap::Parser;
use moss::signal::inhibit;
use thiserror::Error;
use thread_priority::{NormalThreadSchedulePolicy, ThreadPriority, ThreadSchedulePolicy, thread_native_id};

#[derive(Debug, Parser)]
#[command(about = "Build ... TODO")]
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
        ..
    } = command;

    let mut timing = Timing::default();
    let timer = timing.begin(timing::Kind::Initialize);

    if !output.exists() {
        return Err(Error::MissingOutput(output));
    }

    let builder = Builder::new(&recipe_path, env, profile, ccache, output)?;
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

    println!(
        "Build finished successfully at {}",
        Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );

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
}
