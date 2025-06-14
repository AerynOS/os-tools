// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use stone_recipe::{Script, Upstream, script, tuning};
use thiserror::Error;

pub use self::phase::Phase;
use crate::build::pgo;
use crate::{Macros, Paths, Recipe, architecture::BuildTarget, util};

mod phase;

#[derive(Debug)]
pub struct Job {
    pub target: BuildTarget,
    pub pgo_stage: Option<pgo::Stage>,
    pub phases: BTreeMap<Phase, Script>,
    pub work_dir: PathBuf,
    pub build_dir: PathBuf,
}

impl Job {
    pub fn new(
        target: BuildTarget,
        pgo_stage: Option<pgo::Stage>,
        recipe: &Recipe,
        paths: &Paths,
        macros: &Macros,
        ccache: bool,
    ) -> Result<Self, Error> {
        let build_dir = paths.build().guest.join(target.to_string());
        let work_dir = work_dir(&build_dir, &recipe.parsed.upstreams);

        let phases = phase::list(pgo_stage)
            .into_iter()
            .filter_map(|phase| {
                let result = phase
                    .script(target, pgo_stage, recipe, paths, macros, ccache)
                    .transpose()?;
                Some(result.map(|script| (phase, script)))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            target,
            pgo_stage,
            phases,
            work_dir,
            build_dir,
        })
    }
}

fn work_dir(build_dir: &Path, upstreams: &[Upstream]) -> PathBuf {
    let mut work_dir = build_dir.to_path_buf();

    // Work dir is the first upstream that should be unpacked
    if let Some(upstream) = upstreams.iter().find(|upstream| match upstream {
        Upstream::Plain { unpack, .. } => *unpack,
        Upstream::Git { .. } => true,
    }) {
        match upstream {
            Upstream::Plain {
                uri,
                rename,
                unpack_dir,
                ..
            } => {
                let file_name = util::uri_file_name(uri);
                let rename = rename.as_deref().unwrap_or(file_name);
                let unpack_dir = unpack_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| rename.to_owned());

                work_dir = build_dir.join(unpack_dir);
            }
            Upstream::Git { uri, clone_dir, .. } => {
                let source = util::uri_file_name(uri);
                let target = clone_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| source.to_owned());

                work_dir = build_dir.join(target);
            }
        }
    }

    work_dir
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing arch macros: {0}")]
    MissingArchMacros(String),
    #[error("script")]
    Script(#[from] script::Error),
    #[error("tuning")]
    Tuning(#[from] tuning::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
