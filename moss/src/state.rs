// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::Write;

use chrono::{DateTime, Utc};
use derive_more::{Debug, Display, From, Into};
use tui::{pretty, Styled};

use crate::package;

/// Unique identifier for [`State`]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, From, Into, Display)]
#[debug("{_0:?}")]
pub struct Id(i32);

impl Id {
    /// Return the next sequential Id
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// State types
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
pub enum Kind {
    /// Automatically constructed state
    Transaction,
}

impl TryFrom<String> for Kind {
    type Error = strum::ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Unique identifier for this state
    pub id: Id,
    /// Quick summary for the state (optional)
    pub summary: Option<String>,
    /// Description for the state (optional)
    pub description: Option<String>,
    /// Selections in this state
    pub selections: Vec<Selection>,
    /// Creation timestamp
    pub created: DateTime<Utc>,
    /// Relevant type for this State
    pub kind: Kind,
}

/// The Selection records the presence of a package ID in a [`State`]
/// It also records whether it was selected as a transitive dependency,
/// along with an optional human-readable reason
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub package: package::Id,
    /// Marks whether the package was explicitly installed
    /// by the user, or if it's a "transitive" dependency
    pub explicit: bool,
    pub reason: Option<String>,
}

impl Selection {
    /// Construct a new explicit Selection to indicate user intent
    pub fn explicit(package: package::Id) -> Self {
        Self {
            package,
            explicit: true,
            reason: None,
        }
    }

    /// Construct a new transitive Selection to mark automatic installation
    pub fn transitive(package: package::Id) -> Self {
        Self {
            package,
            explicit: true,
            reason: None,
        }
    }

    /// Record a reason for the Selection entering the state
    pub fn reason(self, reason: impl ToString) -> Self {
        Self {
            reason: Some(reason.to_string()),
            ..self
        }
    }
}

/// Columnar display encapsulation for a [`State`]
pub struct ColumnDisplay<'a>(pub &'a State);

impl pretty::ColumnDisplay for ColumnDisplay<'_> {
    fn get_display_width(&self) -> usize {
        "State ".len() + self.0.id.to_string().len()
    }

    fn display_column(&self, writer: &mut impl Write, _col: pretty::Column, width: usize) {
        let _ = write!(writer, "State {}{:width$}", self.0.id.to_string().bold(), " ");
    }
}
