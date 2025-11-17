// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use container::Container;
use thiserror::Error;

use crate::Paths;

pub fn exec<E>(paths: &Paths, networking: bool, f: impl FnMut() -> Result<(), E>) -> Result<(), Error>
where
    E: std::error::Error + Send + Sync + 'static,
{
    run(paths, networking, f)
}

fn run<E>(paths: &Paths, networking: bool, f: impl FnMut() -> Result<(), E>) -> Result<(), Error>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let rootfs = paths.rootfs().host;
    let artefacts = paths.artefacts();
    let build = paths.build();
    let compiler = paths.ccache();
    let ccache_conf = paths.ccache_config();
    let rustc_wrapper = paths.sccache();
    let recipe = paths.recipe();

    Container::new(rootfs)
        .hostname("boulder")
        .networking(networking)
        .ignore_host_sigint(true)
        .work_dir(&build.guest)
        .bind_rw(&artefacts.host, &artefacts.guest, false)
        .bind_rw(&build.host, &build.guest, false)
        .bind_rw(&compiler.host, &compiler.guest, false)
        .bind_rw(&rustc_wrapper.host, &rustc_wrapper.guest, false)
        .bind_ro(&recipe.host, &recipe.guest, false)
        .bind_ro(&ccache_conf.host, &ccache_conf.guest, true)
        .run::<E>(f)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Container(#[from] container::Error),
}
