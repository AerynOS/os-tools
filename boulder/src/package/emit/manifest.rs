// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::BTreeSet,
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use fs_err as fs;
use moss::{
    package::{Meta, MissingMetaFieldError},
    util,
};
use stone::{StoneDecodedPayload, StoneReadError, StoneWriteError};
use tempfile::NamedTempFile;
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
        let mut output = fs::File::create(self.output_dir.join(format!("manifest.{}.bin", self.arch)))?;

        binary::write(&mut output, &self.packages, &self.build_deps)
    }

    pub fn write_json(&self) -> Result<(), Error> {
        json::write(
            &self.output_dir.join(format!("manifest.{}.jsonc", self.arch)),
            self.recipe,
            &self.packages,
            &self.build_deps,
        )
    }

    /// Verifies this newly built manifest against the provided
    /// manifest at `compare_to` path and returns a [`Verification`]
    /// based on the verification of the two manifests.
    // TODO: Binary manifests do not have layouts. Ideally we would
    // verify that layouts match as well as meta. We are looking
    // to overhaul our binary vs json manifest formats so once
    // that is done, we should revise `verify` to handle a more
    // in-depth comparison
    pub fn verify(&self, compare_to: &Path) -> Result<Verification, Error> {
        // Write the current manifest to a temp file & hash it
        let (current_hash, mut current_temp_file) = {
            let mut temp_file = NamedTempFile::with_prefix("boulder-")?;

            let mut writer = util::Sha256Wrapper::new(&mut temp_file);

            binary::write(&mut writer, &self.packages, &self.build_deps)?;

            let hash = writer.finalize();

            (hash, temp_file)
        };

        // Get the comparison hash & file
        let (compare_to_hash, mut compare_to_file) = {
            let mut file = fs::File::open(compare_to).map_err(Error::OpenManifest)?;

            let hash = util::sha256_hash(&mut file).map_err(Error::HashManifest)?;

            (hash, file)
        };

        // If hashes match, return that match status
        if current_hash == compare_to_hash {
            return Ok(Verification::HashMatch { hash: current_hash });
        }

        // Extracts all meta payloads
        let extract_metas = |reader: &mut std::fs::File| {
            // Reset seek position to read stone payloads
            reader.seek(SeekFrom::Start(0))?;

            let payloads = util::stone_payloads(reader).map_err(Error::ReadStonePayloads)?;

            let metas = payloads
                .iter()
                .filter_map(StoneDecodedPayload::meta)
                .map(|payload| Meta::from_stone_payload(&payload.body).map(OrderedMeta))
                .collect::<Result<BTreeSet<_>, _>>()?;

            Ok(metas) as Result<BTreeSet<_>, Error>
        };

        let current_metas = extract_metas(current_temp_file.as_file_mut())?;
        let compare_to_metas = extract_metas(compare_to_file.file_mut())?;

        if current_metas == compare_to_metas {
            return Ok(Verification::ContentMatch);
        }

        Ok(Verification::Mismatch)
    }
}

/// Verified manifest variant
pub enum Verification {
    /// Manifests do not match
    Mismatch,
    /// Manifests matched via sha256 hash
    HashMatch { hash: String },
    /// Manifests matched via content
    ContentMatch,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("stone binary writer")]
    StoneWriter(#[from] StoneWriteError),
    #[error("encode json")]
    Json(#[from] serde_json::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("open manifest file")]
    OpenManifest(#[source] io::Error),
    #[error("sha256 hash manifest")]
    HashManifest(#[source] io::Error),
    #[error("read stone payloads")]
    ReadStonePayloads(#[source] StoneReadError),
    #[error("manifest missing meta field")]
    ManifestMissingMetaField(#[from] MissingMetaFieldError),
    #[error("Manifest missing meta payload")]
    ManifestMissingMetaPayload,
}

#[derive(Debug, PartialEq, Eq)]
struct OrderedMeta(Meta);

impl PartialOrd for OrderedMeta {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedMeta {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.name.cmp(&other.0.name)
    }
}
