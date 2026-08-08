// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;
use std::{io, path::PathBuf};

use chrono::{Datelike, Utc};
use itertools::Itertools;
use licenses::match_licences;
use moss::{Dependency, util};
use stone_recipe::upstream::SourceUri;
use thiserror::Error;
use tui::{
    Styled,
    dialoguer::{Confirm, theme::ColorfulTheme},
};

use crate::Env;

use self::metadata::Metadata;
use self::monitoring::Monitoring;

mod build;
mod licenses;
mod metadata;
mod monitoring;
pub mod upstream;

pub struct Drafter {
    env: Env,
    source_uris: Vec<SourceUri>,
}

pub enum Confirmation {
    Ask,
    DoNotAsk,
}

pub struct Draft {
    pub stone: String,
    pub monitoring: String,
}

impl Drafter {
    pub fn new(env: Env, source_uris: Vec<SourceUri>) -> Self {
        Self { env, source_uris }
    }

    pub fn run(&self, confirm: Confirmation) -> Result<Draft, Error> {
        let temp_dir = tempfile::tempdir()?;
        let extract_root = temp_dir.as_ref();

        // Fetch upstreams and extract the first one.
        let upstreams = upstream::fetch(&self.env, &self.source_uris)?;

        if upstreams.len() > 1 {
            let proceed = matches!(confirm, Confirmation::DoNotAsk)
                || Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(
                        "Multiple upstreams passed, only the first one will be extracted and analyzed. Continue?",
                    )
                    .default(false)
                    .wait_for_newline(true)
                    .interact()
                    .map_err(|e| Error::Io(e.into()))?;
            if !proceed {
                return Err(Error::Aborted);
            }
        }

        upstream::extract(&self.env, upstreams.first().unwrap(), extract_root)?;

        // Build metadata from extracted upstreams
        let metadata = Metadata::new(upstreams);

        let monitoring = Monitoring::new(&metadata.source.name, &metadata.source.homepage);
        let monitoring_result = monitoring.run()?;

        // Enumerate all extracted files
        let files = util::enumerate_files(extract_root, |_| true)?
            .into_iter()
            .map(|path| File { path, extract_root })
            .collect::<Vec<_>>();

        // Analyze files to determine build system / collect deps
        let build = build::analyze(&files).map_err(Error::AnalyzeBuildSystem)?;

        let licences_dir = &self.env.data_dir.join("licenses");

        let year = Utc::now().year();

        let licenses = format_licenses(match_licences(extract_root, licences_dir).unwrap_or_default());

        // Remove temp extract dir
        drop(temp_dir);

        let build_system = build.detected_system.unwrap_or_else(|| {
            println!(
                "{} | Unhandled build system! - Defaulting to autotools",
                "Warning".yellow()
            );
            build::System::Autotools
        });

        let builddeps = builddeps(build.dependencies);
        let environment = build_system
            .environment()
            .map(|env| format!("environment : |\n    {env}\n"))
            .unwrap_or_default();
        let phases = build_system.phases();
        let options = build_system.options();

        #[rustfmt::skip]
        let template = format!(
"# SPDX-FileCopyrightText: {year} AerynOS Developers
# SPDX-License-Identifier: MPL-2.0

name        : {}
version     : \"{}\"
release     : 1
homepage    : {}
upstreams   :
{}
summary     : UPDATE SUMMARY
description : |
    UPDATE DESCRIPTION
license     : {licenses}
{options}{builddeps}{environment}{phases}",
            metadata.source.name,
            metadata.source.version,
            metadata.source.homepage,
            metadata.upstreams(),
        );

        Ok(Draft {
            stone: template,
            monitoring: monitoring_result,
        })
    }
}

fn builddeps(deps: impl IntoIterator<Item = Dependency>) -> String {
    let deps = deps.into_iter().map(|dep| format!("    - {dep}")).sorted().join("\n");

    if deps.is_empty() {
        String::default()
    } else {
        format!("builddeps   :\n{deps}\n")
    }
}

fn format_licenses(licenses: Vec<String>) -> String {
    let formatted = licenses
        .into_iter()
        .map(|license| format!("    - {license}"))
        .sorted_by(|a, b| {
            // HACK: Ensure -or-later for GNU licenses comes before -only
            //       to match 90% of cases. We need to read the standard license
            //       header to figure out the actual variant.
            if a.contains("-only") {
                std::cmp::Ordering::Greater
            } else if b.contains("-only") {
                std::cmp::Ordering::Less
            } else {
                a.cmp(b)
            }
        })
        .join("\n");
    if formatted.is_empty() {
        "UPDATE LICENSE".to_owned()
    } else {
        format!("\n{formatted}")
    }
}

pub struct File<'a> {
    pub path: PathBuf,
    pub extract_root: &'a Path,
}

impl File<'_> {
    // The depth of a file relative to it's extracted archive
    pub fn depth(&self) -> usize {
        let relative = self.path.strip_prefix(self.extract_root).unwrap_or(&self.path);

        // Subtract 2 so root of archive folder == depth 0
        relative.iter().count().saturating_sub(2)
    }

    pub fn file_name(&self) -> &str {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("analyzing build system")]
    AnalyzeBuildSystem(#[source] build::Error),
    #[error("upstream")]
    Upstream(#[from] crate::upstream::Error),
    #[error("monitoring")]
    Monitoring(#[from] monitoring::Error),
    #[error("licensing")]
    Licenses(#[from] licenses::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("walkdir")]
    WalkDir(#[from] walkdir::Error),
    #[error("operation aborted by user")]
    Aborted,
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_file_depth() {
        let extract_root = Path::new("/tmp/test");

        let file = File {
            path: PathBuf::from("/tmp/test/some_archive/meson.build"),
            extract_root,
        };

        assert_eq!(file.depth(), 0);
    }
}
