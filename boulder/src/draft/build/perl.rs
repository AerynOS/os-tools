// SPDX-FileCopyrightText: Copyright © 2025 Aeryn OS Developers
//
// SPDX-License-Identifier: MPL-2.0

pub mod extutils_makefile {
    use crate::draft::File;
    use crate::draft::build::{Error, Phases, State};

    pub fn phases() -> Phases {
        Phases {
            setup: Some("%perl_setup"),
            build: Some("%make"),
            install: Some("%make_install"),
            check: Some("%make test"),
        }
    }

    pub fn process(state: &mut State<'_>, file: &File<'_>) -> Result<(), Error> {
        if file.file_name() == "Makefile.PL" {
            state.increment_confidence(100);
        }

        Ok(())
    }
}

pub mod module_build {
    use crate::draft::File;
    use crate::draft::build::{Error, Phases, State};

    pub fn phases() -> Phases {
        Phases {
            setup: Some("%perl_module_setup"),
            build: Some("%perl_module_build"),
            install: Some("%perl_module_install"),
            check: None,
        }
    }

    pub fn process(state: &mut State<'_>, file: &File<'_>) -> Result<(), Error> {
        if file.file_name() == "Build.PL" {
            // We prefer Makefile.PL if available
            state.increment_confidence(95);
        }

        Ok(())
    }
}
