// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;
use std::{io, path::PathBuf};

use fs_err as fs;
use itertools::Itertools;
use licenses::match_licences;
use moss::Dependency;
use thiserror::Error;
use tui::Styled;
use url::Url;

use crate::util;

use self::metadata::Metadata;
use self::monitoring::Monitoring;
use self::upstream::Upstream;

mod build;
mod licenses;
mod metadata;
mod monitoring;
mod upstream;

pub struct Drafter {
    upstreams: Vec<Url>,
    datadir: PathBuf,
}

pub struct Draft {
    pub stone: String,
    pub monitoring: String,
}

impl Drafter {
    pub fn new(upstreams: Vec<Url>, datadir: PathBuf) -> Self {
        Self { upstreams, datadir }
    }

    pub fn run(&self) -> Result<Draft, Error> {
        // TODO: Use tempdir
        let extract_root = PathBuf::from("/tmp/boulder-new");

        // Fetch and extract all upstreams
        let extracted = upstream::fetch_and_extract(&self.upstreams, &extract_root)?;

        // Build metadata from extracted upstreams
        let metadata = Metadata::new(extracted);

        let monitoring = Monitoring::new(&metadata.source.name, &metadata.source.homepage);
        let monitoring_result = monitoring.run()?;

        // Enumerate all extracted files
        let files = util::enumerate_files(&extract_root, |_| true)?
            .into_iter()
            .map(|path| File {
                path,
                extract_root: &extract_root,
            })
            .collect::<Vec<_>>();

        // Analyze files to determine build system / collect deps
        let build = build::analyze(&files).map_err(Error::AnalyzeBuildSystem)?;

        let licences_dir = &self.datadir.join("licenses");

        let licenses = format_licenses(match_licences(&extract_root, licences_dir).unwrap_or_default());

        // Remove temp extract dir
        fs::remove_dir_all(extract_root)?;

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
"#
# SPDX-FileCopyrightText: © 2020-2025 Serpent OS Developers
#
# SPDX-License-Identifier: MPL-2.0
#
name        : {}
version     : {}
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
    Upstream(#[from] upstream::Error),
    #[error("monitoring")]
    Monitoring(#[from] monitoring::Error),
    #[error("licensing")]
    Licenses(#[from] licenses::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("walkdir")]
    WalkDir(#[from] jwalk::Error),
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
