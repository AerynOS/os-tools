use std::path::Path;

use fs_err as fs;
use moss::{Dependency, dependency};
use regex::Regex;

// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use crate::draft::File;
use crate::draft::build::{Error, Phases, State};

pub fn phases() -> Phases {
    Phases {
        setup: Some("%configure"),
        build: Some("%make"),
        install: Some("%make_install"),
        check: None,
    }
}

pub fn process(state: &mut State<'_>, file: &File<'_>) -> Result<(), Error> {
    // Depth too great
    if file.depth() > 0 {
        return Ok(());
    }

    match file.file_name() {
        "configure.ac" => {
            state.increment_confidence(10);
            scan_autotools(state, &file.path)?;
        }
        "configure" | "Makefile.am" | "Makefile" => {
            state.increment_confidence(10);
        }
        _ => {}
    }

    Ok(())
}

fn scan_autotools(state: &mut State<'_>, path: &Path) -> Result<(), Error> {
    let regex_pkgconfig =
        Regex::new(r"PKG_CHECK_MODULES\s?\(\s?\[([A-Za-z_]+)\s?\]\s?,\s?\[\s?(\s?[A-Za-z0-9\-_+]+)\s?]")?;

    let contents = fs::read_to_string(path)?;

    // Check all meson dependency() calls
    for captures in regex_pkgconfig.captures_iter(&contents) {
        if let Some(capture) = captures.get(2) {
            let name = capture.as_str().to_owned();

            state.add_dependency(Dependency {
                kind: dependency::Kind::PkgConfig,
                name,
            });
        }
    }

    Ok(())
}
