// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io;

use container::Container;
use nix::sys::signal::Signal;
use thiserror::Error;

use crate::Paths;

pub fn exec(
    paths: &Paths,
    networking: bool,
    f: impl FnMut() -> Result<(), ExecError>,
) -> Result<(), Error> {
    run(paths, networking, f)
}

fn run(
    paths: &Paths,
    networking: bool,
    f: impl FnMut() -> Result<(), ExecError>,
) -> Result<(), Error> {
    let rootfs = paths.rootfs().host;
    let artefacts = paths.artefacts();
    let build = paths.build();
    let compiler = paths.ccache();
    let recipe = paths.recipe();

    Container::new(rootfs)
        .hostname("boulder")
        .networking(networking)
        .ignore_host_sigint(true)
        .work_dir(&build.guest)
        .bind_rw(&artefacts.host, &artefacts.guest)
        .bind_rw(&build.host, &build.guest)
        .bind_rw(&compiler.host, &compiler.guest)
        .bind_ro(&recipe.host, &recipe.guest)
        .run::<ExecError>(f)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Container(#[from] container::Error),
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("failed with status code {0}")]
    Code(i32),
    #[error("stopped by signal {}", .0.as_str())]
    Signal(Signal),
    #[error("stopped by unknown signal")]
    UnknownSignal,
    #[error(transparent)]
    Nix(#[from] nix::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}
