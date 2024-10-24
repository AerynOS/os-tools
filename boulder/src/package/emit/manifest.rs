// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, io, path::PathBuf};

use stone::StoneWriteError;
use thiserror::Error;

use crate::{Architecture, Paths, Recipe};

use super::Package;

mod binary;
mod json;

#[derive(Debug)]
pub struct Manifest<'a> {
    recipe: &'a Recipe,
    arch: Architecture,
    output_dir: PathBuf,
    build_deps: BTreeSet<String>,
    packages: BTreeSet<&'a Package<'a>>,
}

impl<'a> Manifest<'a> {
    pub fn new(paths: &Paths, recipe: &'a Recipe, arch: Architecture) -> Self {
        let output_dir = paths.artefacts().guest;

        let build_deps = recipe
            .parsed
            .build
            .build_deps
            .iter()
            .chain(&recipe.parsed.build.check_deps)
            .cloned()
            .collect();

        Self {
            recipe,
            output_dir,
            arch,
            build_deps,
            packages: BTreeSet::new(),
        }
    }

    pub fn add_package(&mut self, package: &'a Package<'_>) {
        self.packages.insert(package);
    }

    pub fn write_binary(&self) -> Result<(), Error> {
        binary::write(
            &self.output_dir.join(format!("manifest.{}.bin", self.arch)),
            &self.packages,
            &self.build_deps,
        )
    }

    pub fn write_json(&self) -> Result<(), Error> {
        json::write(
            &self.output_dir.join(format!("manifest.{}.jsonc", self.arch)),
            self.recipe,
            &self.packages,
            &self.build_deps,
        )
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("stone binary writer")]
    StoneWriter(#[from] StoneWriteError),
    #[error("encode json")]
    Json(#[from] serde_json::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
