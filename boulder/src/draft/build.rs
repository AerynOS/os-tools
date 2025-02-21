// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::collections::{BTreeMap, BTreeSet};
use std::{fmt, num::NonZeroU64};

use moss::Dependency;

use super::File;

mod autotools;
mod cargo;
mod cmake;
mod meson;
mod python;
mod ruby;

pub type Error = Box<dyn std::error::Error>;

/// A build system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum System {
    Autotools,
    Cargo,
    Cmake,
    Meson,
    PythonPep517,
    PythonSetupTools,
    RubyGem,
    RubyTarball,
}

impl System {
    const ALL: &'static [Self] = &[
        Self::Autotools,
        Self::Cargo,
        Self::Cmake,
        Self::Meson,
        Self::PythonPep517,
        Self::PythonSetupTools,
        Self::RubyGem,
        Self::RubyTarball,
    ];

    pub fn environment(&self) -> Option<&'static str> {
        match self {
            System::Autotools => None,
            System::Cargo => None,
            System::Cmake => None,
            System::Meson => None,
            System::PythonPep517 => None,
            System::PythonSetupTools => None,
            System::RubyGem => None,
            System::RubyTarball => None,
        }
    }

    pub fn phases(&self) -> Phases {
        match self {
            System::Autotools => autotools::phases(),
            System::Cargo => cargo::phases(),
            System::Cmake => cmake::phases(),
            System::Meson => meson::phases(),
            System::PythonPep517 => python::pep517::phases(),
            System::PythonSetupTools => python::setup_tools::phases(),
            System::RubyGem => ruby::gemfile::phases(),
            System::RubyTarball => ruby::tarball::phases(),
        }
    }

    /// return specific options for a build system
    pub fn options(&self) -> Options {
        match self {
            System::Cargo => Options { networking: true },
            _ => Options { networking: false },
        }
    }

    fn process(&self, state: &mut State<'_>, file: &File<'_>) -> Result<(), Error> {
        match self {
            System::Autotools => autotools::process(state, file),
            System::Cargo => cargo::process(state, file),
            System::Cmake => cmake::process(state, file),
            System::Meson => meson::process(state, file),
            System::PythonPep517 => python::pep517::process(state, file),
            System::PythonSetupTools => python::setup_tools::process(state, file),
            System::RubyGem => ruby::gemfile::process(state, file),
            System::RubyTarball => ruby::tarball::process(state, file),
        }
    }
}

/// Commands to run for each build phase of the [`System`]
pub struct Phases {
    pub setup: Option<&'static str>,
    pub build: Option<&'static str>,
    pub install: Option<&'static str>,
    pub check: Option<&'static str>,
}

impl fmt::Display for Phases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fmt = |name, value| {
            if let Some(value) = value {
                writeln!(f, "{name:<12}: |\n    {value}")
            } else {
                Ok(())
            }
        };
        fmt("setup", self.setup)?;
        fmt("build", self.build)?;
        fmt("install", self.install)?;
        fmt("check", self.check)
    }
}

pub struct Options {
    // Enforced networking for the build
    pub networking: bool,
}

impl fmt::Display for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fmt = |name, value| {
            if let Some(value) = value {
                writeln!(f, "{name:<12}: {value}")
            } else {
                Ok(())
            }
        };
        fmt("networking", self.networking.then_some(true))
    }
}

/// State passed to each system when processing paths
struct State<'a> {
    /// Any dependencies that need to be recorded
    dependencies: &'a mut BTreeSet<Dependency>,
    /// Total confidence level of the current build [`System`]
    confidence: u64,
}

impl State<'_> {
    /// Increase the confidence that this project uses the current build [`System`]
    pub fn increment_confidence(&mut self, amount: u64) {
        self.confidence += amount;
    }

    /// Add a dependency to output in `builddeps`
    pub fn add_dependency(&mut self, dependency: Dependency) {
        self.dependencies.insert(dependency);
    }
}

/// Analysis results from [`analyze`]
pub struct Analysis {
    /// The detected build [`System`], if any
    pub detected_system: Option<System>,
    /// All detected dependencies
    pub dependencies: BTreeSet<Dependency>,
}

/// Analyze the provided paths to determine which build [`System`]
/// the project uses and any dependencies that are identified
pub fn analyze(files: &[File<'_>]) -> Result<Analysis, Error> {
    let mut dependencies = BTreeSet::new();
    let mut confidences = BTreeMap::new();

    for system in System::ALL {
        let mut state = State {
            dependencies: &mut dependencies,
            confidence: 0,
        };

        for path in files {
            system.process(&mut state, path)?;
        }

        if let Some(confidence) = NonZeroU64::new(state.confidence) {
            confidences.insert(*system, confidence);
        }
    }

    let detected_system = confidences
        .into_iter()
        .max_by_key(|(_, confidence)| *confidence)
        .map(|(system, _)| system);

    Ok(Analysis {
        detected_system,
        dependencies,
    })
}
