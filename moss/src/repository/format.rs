// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::fmt;

use serde::{Deserialize, Serialize};

pub use self::identifier::{Identifier, ScopedIdentifier};
pub use self::root_index::RootIndex;

pub mod identifier;
pub mod root_index;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Legacy,
    V0,
    #[serde(untagged)]
    Unsupported(String),
}

impl Format {
    pub const LATEST: Self = Self::V0;
}

impl From<&str> for Format {
    fn from(value: &str) -> Self {
        match value {
            "legacy" => Format::Legacy,
            "v0" => Format::V0,
            _ => Format::Unsupported(value.to_owned()),
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Format::Legacy => "legacy".fmt(f),
            Format::V0 => "v0".fmt(f),
            Format::Unsupported(s) => s.fmt(f),
        }
    }
}
