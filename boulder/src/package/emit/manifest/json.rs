// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
};

use fs_err::File;
use itertools::Itertools;
use serde::Serialize;

use super::Error;
use crate::{Recipe, package::emit};

pub fn write(
    path: &Path,
    recipe: &Recipe,
    packages: &BTreeSet<&emit::Package<'_>>,
    build_deps: &BTreeSet<String>,
) -> Result<(), Error> {
    let packages = packages
        .iter()
        .map(|package| {
            let name = package.name.to_owned();

            let build_depends = build_deps.iter().cloned().collect();
            let mut depends = package
                .analysis
                .dependencies()
                .map(ToString::to_string)
                .chain(package.definition.run_deps.clone())
                .collect::<Vec<_>>();
            depends.sort();
            depends.dedup();

            let provides = package.analysis.providers().map(ToString::to_string).collect();

            let files = package
                .analysis
                .paths
                .iter()
                .map(|p| format!("/usr/{}", p.layout.entry.target()))
                .sorted()
                .collect();

            let package = Package {
                build_depends,
                depends,
                files,
                name: name.clone(),
                provides,
            };

            (name, package)
        })
        .collect();

    let content = Content {
        manifest_version: "0.2".to_owned(),
        packages,
        source_name: recipe.parsed.source.name.clone(),
        source_release: recipe.parsed.source.release.to_string(),
        source_version: recipe.parsed.source.version.clone(),
    };

    let mut file = File::create(path)?;

    writeln!(
        &mut file,
        "/** Human readable report. This is not consumed by boulder */"
    )?;

    let mut serializer =
        serde_json::Serializer::with_formatter(&mut file, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
    content.serialize(&mut serializer)?;

    writeln!(&mut file)?;

    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct Content {
    manifest_version: String,
    packages: BTreeMap<String, Package>,
    source_name: String,
    source_release: String,
    source_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct Package {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    build_depends: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<String>,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    provides: Vec<String>,
}
