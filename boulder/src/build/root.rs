// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;
use std::io;

use fs_err as fs;
use moss::{repository, runtime, Installation};
use stone_recipe::{tuning::Toolchain, Upstream};
use thiserror::Error;

use crate::build::Builder;
use crate::{container, timing, util, Timing};

pub fn populate(
    builder: &Builder,
    repositories: repository::Map,
    timing: &mut Timing,
    initialize_timer: timing::Timer,
    update_repos: bool,
) -> Result<(), Error> {
    let packages = packages(builder);

    let rootfs = builder.paths.rootfs().host;

    // Create the moss client
    let installation = Installation::open(&builder.env.moss_dir, None)?;
    let mut moss_client =
        moss::Client::with_explicit_repositories("boulder", installation, repositories)?.ephemeral(rootfs)?;

    if update_repos {
        runtime::block_on(moss_client.refresh_repositories())?;
        println!();
    } else {
        // Ensure all configured repos have been initialized (important since users
        // might add profile configs from an editor)
        if runtime::block_on(moss_client.ensure_repos_initialized())? > 0 {
            println!();
        }
    }

    timing.finish(initialize_timer);

    // Install packages
    let install_timing = moss_client.install(&packages, true)?;

    timing.record(timing::Populate::Resolve, install_timing.resolve);
    timing.record(timing::Populate::Fetch, install_timing.fetch);
    timing.record(timing::Populate::Blit, install_timing.blit);

    Ok(())
}

pub fn clean(builder: &Builder) -> Result<(), Error> {
    // Dont't need to clean if it doesn't exist
    if !builder.paths.rootfs().host.exists() {
        return Ok(());
    }

    // We remove certain paths inside the container so we don't
    // get permissions error if this is a rootless build
    // and there's subuid mappings into the user namespace
    container::exec(&builder.paths, false, || {
        // Remove install dir
        let install_dir = builder.paths.install().guest;
        if install_dir.exists() {
            fs::remove_dir_all(install_dir)?;
        }

        for target in &builder.targets {
            for job in &target.jobs {
                if job.build_dir.exists() {
                    // Remove build dir
                    fs::remove_dir_all(&job.build_dir)?;
                }
            }
        }

        Ok(()) as io::Result<_>
    })?;

    // Now we can safely recreate the rootfs
    util::recreate_dir(&builder.paths.rootfs().host)?;

    Ok(())
}

fn packages(builder: &Builder) -> Vec<&str> {
    let mut packages = BASE_PACKAGES.to_vec();

    match builder.recipe.parsed.options.toolchain {
        Toolchain::Llvm => packages.extend(LLVM_PACKAGES),
        Toolchain::Gnu => packages.extend(GNU_PACKAGES),
    }

    if builder.recipe.parsed.emul32 {
        packages.extend(BASE32_PACKAGES);

        match builder.recipe.parsed.options.toolchain {
            Toolchain::Llvm => packages.extend(LLVM32_PACKAGES),
            Toolchain::Gnu => packages.extend(GNU32_PACKAGES),
        }
    }

    if builder.recipe.parsed.mold {
        packages.extend(MOLD_PACKAGES);
    }

    if builder.ccache {
        packages.extend(CCACHE_PACKAGES);
    }

    packages.extend(
        builder.recipe.parsed.build.build_deps.iter().map(String::as_str).chain(
            builder
                .recipe
                .parsed
                .profiles
                .iter()
                .flat_map(|kv| kv.value.build_deps.iter().map(String::as_str)),
        ),
    );
    packages.extend(
        builder.recipe.parsed.build.check_deps.iter().map(String::as_str).chain(
            builder
                .recipe
                .parsed
                .profiles
                .iter()
                .flat_map(|kv| kv.value.check_deps.iter().map(String::as_str)),
        ),
    );

    for upstream in &builder.recipe.parsed.upstreams {
        if let Upstream::Plain { uri, .. } = upstream {
            let path = uri.path();

            if let Some((_, ext)) = path.rsplit_once('.') {
                match ext {
                    "xz" => {
                        packages.extend(["binary(tar)", "binary(xz)"]);
                    }
                    "zst" => {
                        packages.extend(["binary(tar)", "binary(zstd)"]);
                    }
                    "bz2" => {
                        packages.extend(["binary(tar)", "binary(bzip2)"]);
                    }
                    "gz" => {
                        packages.extend(["binary(tar)", "binary(gzip)"]);
                    }
                    "zip" => {
                        packages.push("binary(unzip)");
                    }
                    "rpm" => {
                        packages.extend(["binary(rpm2cpio)", "cpio"]);
                    }
                    "deb" => {
                        packages.push("binary(ar)");
                    }
                    _ => {}
                }
            }
        }
    }

    // Dependencies from all scripts in the builder
    let extra_deps = builder.extra_deps();

    packages
        .into_iter()
        .chain(extra_deps)
        // Remove dupes
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

const BASE_PACKAGES: &[&str] = &[
    "bash",
    "boulder",
    "coreutils",
    "dash",
    "diffutils",
    "findutils",
    "gawk",
    "glibc-devel",
    "grep",
    "layout",
    "libarchive",
    "linux-headers",
    "pkgconf",
    "sed",
    "util-linux",
    // Needed for chroot
    "binary(git)",
    "binary(nano)",
    "binary(vim)",
    "binary(ps)",
];
const BASE32_PACKAGES: &[&str] = &["glibc-32bit-devel"];

const GNU_PACKAGES: &[&str] = &["binutils", "gcc", "g++"];
const GNU32_PACKAGES: &[&str] = &["gcc-32bit", "libstdc++-32bit-devel"];

const LLVM_PACKAGES: &[&str] = &["clang"];
const LLVM32_PACKAGES: &[&str] = &["clang-32bit", "libcxx-32bit-devel"];

const MOLD_PACKAGES: &[&str] = &["binary(mold)"];

const CCACHE_PACKAGES: &[&str] = &["binary(ccache)", "binary(sccache)"];

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),
    #[error("moss client")]
    MossClient(#[from] moss::client::Error),
    #[error("moss install")]
    MossInstall(#[from] moss::client::install::Error),
    #[error("moss installation")]
    MossInstallation(#[from] moss::installation::Error),
    #[error("container")]
    Container(#[from] container::Error),
}
