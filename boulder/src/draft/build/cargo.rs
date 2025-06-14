// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use crate::draft::File;
use crate::draft::build::{Error, Phases, State};

pub fn phases() -> Phases {
    Phases {
        setup: Some("%cargo_fetch"),
        build: Some("%cargo_build"),
        install: Some("%cargo_install"),
        check: Some("%cargo_test"),
    }
}

pub fn process(state: &mut State<'_>, file: &File<'_>) -> Result<(), Error> {
    if file.file_name() == "Cargo.toml" {
        state.increment_confidence(100);
    }

    Ok(())
}
