// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{borrow::Cow, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// A URL safe identifier which matches `[a-zA-Z0-9][a-zA-Z0-9-]*`
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, derive_more::Display, derive_more::AsRef,
)]
#[serde(try_from = "Cow<'_, str>")]
pub struct Identifier(String);

impl Identifier {
    pub fn new(s: &str) -> Result<Self, InvalidIdentifierError> {
        if !s.is_empty() && s.as_bytes()[0] != b'-' && s.chars().all(|char| char.is_alphanumeric() || char == '-') {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidIdentifierError {
                identifier: s.to_owned(),
            })
        }
    }
}

impl TryFrom<&'_ str> for Identifier {
    type Error = InvalidIdentifierError;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for Identifier {
    type Error = InvalidIdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl TryFrom<Cow<'_, str>> for Identifier {
    type Error = InvalidIdentifierError;

    fn try_from(value: Cow<'_, str>) -> Result<Self, Self::Error> {
        Self::new(value.as_ref())
    }
}

#[derive(Debug, Snafu)]
#[snafu(display("invalid identifier `{identifier}`, must match [a-zA-Z0-9][a-zA-Z0-9-]*"))]
pub struct InvalidIdentifierError {
    identifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Cow<'_, str>", into = "String")]
pub enum ScopedIdentifier {
    Stream(Identifier),
    Tag(Identifier),
    History(Identifier),
}

impl fmt::Display for ScopedIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopedIdentifier::Stream(identifier) => write!(f, "stream/{identifier}"),
            ScopedIdentifier::Tag(identifier) => write!(f, "tag/{identifier}"),
            ScopedIdentifier::History(identifier) => write!(f, "history/{identifier}"),
        }
    }
}

impl FromStr for ScopedIdentifier {
    type Err = ParseScopedIdentifierError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('/') {
            Some(("stream", identifier)) => Ok(Self::Stream(Identifier::new(identifier)?)),
            Some(("tag", identifier)) => Ok(Self::Tag(Identifier::new(identifier)?)),
            Some(("history", identifier)) => Ok(Self::History(Identifier::new(identifier)?)),
            Some((scope, _)) => Err(ParseScopedIdentifierError::UnknownScope {
                scope: scope.to_owned(),
            }),
            None => Err(ParseScopedIdentifierError::MissingScopeSeparator),
        }
    }
}

impl TryFrom<&'_ str> for ScopedIdentifier {
    type Error = ParseScopedIdentifierError;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<String> for ScopedIdentifier {
    type Error = ParseScopedIdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<Cow<'_, str>> for ScopedIdentifier {
    type Error = ParseScopedIdentifierError;

    fn try_from(value: Cow<'_, str>) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ScopedIdentifier> for String {
    fn from(value: ScopedIdentifier) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Snafu)]
pub enum ParseScopedIdentifierError {
    #[snafu(display("missing scope separator '/'"))]
    MissingScopeSeparator,
    #[snafu(display("unknown scope {scope}"))]
    UnknownScope { scope: String },
    #[snafu(transparent)]
    InvalidIdentifier { source: InvalidIdentifierError },
}
