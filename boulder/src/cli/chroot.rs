// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::PathBuf, process};

use boulder::{
    Env, Macros, Paths, Recipe,
    architecture::{self, BuildTarget},
    build, container, macros, recipe,
};
use clap::Parser;
use fs_err as fs;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(about = "Chroot into the build environment")]
pub struct Command {
    #[arg(default_value = "./stone.yaml", help = "Path to recipe file")]
    recipe: PathBuf,
}

pub fn handle(command: Command, env: Env) -> Result<(), Error> {
    let Command { recipe: recipe_path } = command;

    let recipe = Recipe::load(recipe_path)?;
    let macros = Macros::load(&env)?;
    let paths = Paths::new(&recipe, env.cache_dir, "/mason", ".")?;

    let rootfs = paths.rootfs().host;

    // Has rootfs been setup?
    if !rootfs.join("usr").exists() {
        return Err(Error::MissingRootFs);
    }

    // Generate a script so we can inject a .profile
    // to the container environment with all actions
    // and definitions
    //
    // The phase doesn't matter, but we use `prepare`
    // since it uses hardcoded content that's always
    // available to create a script from
    let script = build::job::Phase::Prepare
        .script(
            BuildTarget::Native(architecture::host()),
            None,
            &recipe,
            &paths,
            &macros,
            false,
        )
        .map_err(Error::BuildScript)?
        .expect("script always available for prepare phase");
    let profile = &build::format_profile(&script);

    let home = &paths.build().guest;

    container::exec(&paths, recipe.parsed.options.networking, || {
        fs::write(home.join(".profile"), profile)?;

        let mut child = process::Command::new("/bin/bash")
            .arg("--login")
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/usr/sbin")
            .env("TERM", "xterm-256color")
            .spawn()?;

        child.wait()?;

        Ok(()) as io::Result<_>
    })?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("build root doesn't exist, make sure to run build first")]
    MissingRootFs,
    #[error("container")]
    Container(#[from] container::Error),
    #[error("macros")]
    Macros(#[from] macros::Error),
    #[error("build script")]
    BuildScript(#[source] build::job::Error),
    #[error("recipe")]
    Recipe(#[from] recipe::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
