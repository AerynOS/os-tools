// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use derive_more::{AsRef, Debug, Display, From, Into};
use itertools::Itertools;

pub use self::meta::{Meta, MissingMetaFieldError, Name};

pub mod meta;
pub mod render;

/// Unique ID of a [`Package`]
#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, From, Into, AsRef, Display)]
#[as_ref(forward)]
#[debug("{_0:?}")]
pub struct Id(String);

impl From<Id> for meta::Id {
    fn from(id: Id) -> Self {
        meta::Id(id.0)
    }
}

impl From<meta::Id> for Id {
    fn from(id: meta::Id) -> Self {
        Self(id.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: Id,
    pub meta: Meta,
    pub flags: Flags,
}

impl PartialOrd for Package {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Package {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.meta
            .source_release
            .cmp(&other.meta.source_release)
            .reverse()
            .then_with(|| self.meta.build_release.cmp(&other.meta.build_release).reverse())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// Package is available for installation.
    pub available: bool,
    /// Package is already installed.
    pub installed: bool,
    /// Available as from-source build.
    pub source: bool,
    /// Package is explicitly installed (use with [`Flags::installed`]).
    pub explicit: bool,
}

impl Flags {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy of [`Flags`] with available set to true.
    pub fn with_available(&self) -> Self {
        Self {
            available: true,
            ..*self
        }
    }

    /// Returns a copy of [`Flags`] with installed set to true.
    pub fn with_installed(&self) -> Self {
        Self {
            installed: true,
            ..*self
        }
    }

    /// Returns a copy of [`Flags`] with source set to true.
    pub fn with_source(&self) -> Self {
        Self { source: true, ..*self }
    }

    /// Returns a copy of [`Flags`] with explicit set to true.
    pub fn with_explicit(&self) -> Self {
        Self {
            explicit: true,
            ..*self
        }
    }

    /// Returns whether this flag set contains another flag set.
    pub fn contains(&self, other: Self) -> bool {
        (self.bits() & other.bits()) == other.bits()
    }

    fn bits(&self) -> u32 {
        (self.available as u32)
            | ((self.installed as u32) << 1)
            | ((self.source as u32) << 2)
            | ((self.explicit as u32) << 3)
    }
}

/// Iterate packages in sorted order
pub struct Sorted<I>(I);

impl<I> Sorted<I> {
    pub fn new(iter: I) -> Self {
        Self(iter)
    }
}

/// Iterate in sorted order
impl<I, T> IntoIterator for Sorted<I>
where
    I: IntoIterator<Item = T>,
    T: Ord,
{
    type Item = T;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().sorted()
    }
}

/// A package being updated from `old` to `new`
pub struct Update<'a> {
    pub old: &'a Package,
    pub new: &'a Package,
}
