// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

pub mod pep517 {
    use crate::draft::build::{Error, Phases, State};
    use crate::draft::File;

    pub fn phases() -> Phases {
        Phases {
            setup: None,
            build: Some("%pyproject_build"),
            install: Some("%pyproject_install"),
            check: None,
        }
    }

    pub fn process(state: &mut State, file: &File) -> Result<(), Error> {
        match file.file_name() {
            "pyproject.toml" | "setup.cfg" => state.increment_confidence(100),
            _ => {}
        }

        Ok(())
    }
}

pub mod setup_tools {
    use crate::draft::build::{Error, Phases, State};
    use crate::draft::File;

    pub fn phases() -> Phases {
        Phases {
            setup: None,
            build: Some("%python_build"),
            install: Some("%python_install"),
            check: None,
        }
    }

    pub fn process(state: &mut State, file: &File) -> Result<(), Error> {
        if file.file_name() == "setup.py" {
            state.increment_confidence(100);
        }

        Ok(())
    }
}
