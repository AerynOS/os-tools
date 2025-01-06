// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Moss (v1) dependency and provider types
//!
//! This module currently defines the dependency and provider kinds and layouts,
//! but in a future version of moss each version-specific implementation will live
//! in a nested module, i.e `crate::dependency::v1`
//!
//! A Dependency, as expected, is a simple tagged string defining what *kind* of thing
//! a package depends on, and what that thing is. More specifically this could be a shared
//! library dependency on `libz.so.1`, expressed as a [`Kind::SharedLibrary`] dependency
//! with target `libz.so.1(x86_64)`.
//!
//! As one might expect, a [`Provider`] is the inverse of a dependency. It is used to record
//! the capabilities of a package such that others may depend on it through resolution.
//!
//! We acknowledge the current dependency format, while powerful, does not allow expressing
//! relationship constraints. This was a deliberate decision due to the rolling nature of
//! Serpent OS, however more expressive dependency types will be introduced in the next major
//! stone format.
use std::str::FromStr;

use derive_more::Display;
use stone::StonePayloadMetaDependency;
use thiserror::Error;

/// Every dependency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum Kind {
    /// Name based dependency
    #[strum(serialize = "name")]
    PackageName,

    /// Shared library (soname)
    #[strum(serialize = "soname")]
    SharedLibrary,

    /// Exported pkg-config provider
    PkgConfig,

    /// PT_INTERP, or specialist shell support
    Interpreter,

    /// CMake module dependency
    CMake,

    /// Python dependency
    Python,

    /// Executable in /usr/bin
    Binary,

    /// Executable in /usr/sbin
    #[strum(serialize = "sysbinary")]
    SystemBinary,

    /// Exported 32-bit pkgconfig provider
    PkgConfig32,
}

/// Convert payload dependency types to our internal representation
impl From<StonePayloadMetaDependency> for Kind {
    fn from(dependency: StonePayloadMetaDependency) -> Self {
        match dependency {
            StonePayloadMetaDependency::PackageName => Kind::PackageName,
            StonePayloadMetaDependency::SharedLibrary => Kind::SharedLibrary,
            StonePayloadMetaDependency::PkgConfig => Kind::PkgConfig,
            StonePayloadMetaDependency::Interpreter => Kind::Interpreter,
            StonePayloadMetaDependency::CMake => Kind::CMake,
            StonePayloadMetaDependency::Python => Kind::Python,
            StonePayloadMetaDependency::Binary => Kind::Binary,
            StonePayloadMetaDependency::SystemBinary => Kind::SystemBinary,
            StonePayloadMetaDependency::PkgConfig32 => Kind::PkgConfig32,
        }
    }
}

/// Convert our [`Kind`] into a [`Dependency]``
impl From<Kind> for StonePayloadMetaDependency {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::PackageName => Self::PackageName,
            Kind::SharedLibrary => Self::SharedLibrary,
            Kind::PkgConfig => Self::PkgConfig,
            Kind::Interpreter => Self::Interpreter,
            Kind::CMake => Self::CMake,
            Kind::Python => Self::Python,
            Kind::Binary => Self::Binary,
            Kind::SystemBinary => Self::SystemBinary,
            Kind::PkgConfig32 => Self::PkgConfig32,
        }
    }
}

/// A Dependency in moss is simplistic in that it only contains
/// a target and a Kind, ie. `pkgconfig(zlib)`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
#[display("{kind}({name})")]
pub struct Dependency {
    /// Specific type of dependency
    pub kind: Kind,

    /// Bare target (i.e. `libz.so.1(x86_64)`)
    pub name: String,
}

impl Dependency {
    /// Construct a dependency from a specially formatted string
    ///
    /// # Arguments
    ///
    /// * `name` - The formatted name, as seen in `stone.yml`
    ///
    /// # Examples
    ///
    /// ```
    ///     use moss::Dependency;
    ///     let dep = Dependency::from_name("pkgconfig(zlib)").unwrap();
    /// ```
    pub fn from_name(name: &str) -> Result<Self, ParseError> {
        if name.contains('(') {
            Dependency::from_str(name)
        } else {
            Ok(Dependency {
                kind: Kind::PackageName,
                name: name.to_owned(),
            })
        }
    }
}

/// Partial ordering comparator for dependencies
impl PartialOrd for Dependency {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordering comparator for dependencies
impl Ord for Dependency {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_string().cmp(&other.to_string())
    }
}

/// Build a Dependency from a string
impl FromStr for Dependency {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, name) = parse(s)?;

        Ok(Self { kind, name })
    }
}

/// Ditto
impl TryFrom<String> for Dependency {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(value.as_str())
    }
}

/// A provider is the inverse of a [`Dependency`] - providing the matching requirement
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
#[display("{kind}({name})")]
pub struct Provider {
    /// Specific type of dependency
    pub kind: Kind,

    /// Bare target (i.e. `libz.so.1(x86_64)`)
    pub name: String,
}

impl Provider {
    /// Construct a Provider from a specially formatted string
    ///
    /// Identical in behaviour to [`Dependency::from_name`]
    pub fn from_name(name: &str) -> Result<Self, ParseError> {
        if name.contains('(') {
            Provider::from_str(name)
        } else {
            Ok(Provider {
                kind: Kind::PackageName,
                name: name.to_owned(),
            })
        }
    }
}

/// Partial ordering comparator for Provider
impl PartialOrd for Provider {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordering comparator for Provider
impl Ord for Provider {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_string().cmp(&other.to_string())
    }
}

/// Build a Provider from a String
impl FromStr for Provider {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, name) = parse(s)?;

        Ok(Self { kind, name })
    }
}

/// Ditto
impl TryFrom<String> for Provider {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(value.as_str())
    }
}

/// Parse the [`Kind`] of dependency or provider from the string
fn parse(s: &str) -> Result<(Kind, String), ParseError> {
    let (kind, rest) = s.split_once('(').ok_or(ParseError(s.to_owned()))?;

    if !rest.ends_with(')') {
        return Err(ParseError(s.to_owned()));
    }

    let kind = kind.parse::<Kind>().map_err(|e| ParseError(e.to_string()))?;
    // Safe since we checked `ends_with(')')`
    let name = rest[0..rest.len() - 1].to_string();

    Ok((kind, name))
}

/// Parsing error for dependency and provider APIs
#[derive(Debug, Error)]
#[error("Invalid dependency type: {0}")]
pub struct ParseError(String);
