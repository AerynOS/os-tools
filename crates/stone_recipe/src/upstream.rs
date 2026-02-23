// SPDX-FileCopyrightText: Copyright © 2020-2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{borrow::Borrow, collections::BTreeMap, fmt::Display, path::PathBuf, str::FromStr};

use crate::serde_util::{default_true, stringy_bool};
use serde::Deserialize;
use url::Url;

/// Prefix applied to URLs to report they point to a Git repository.
pub static GIT_PREFIX: &str = "git|";

#[derive(Debug, Clone)]
pub struct Upstream {
    pub url: Url,
    pub unpack_dir: Option<PathBuf>,
    pub props: Props,
}

impl<'de> Deserialize<'de> for Upstream {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        struct SerdeProps {
            destdir: Option<PathBuf>,
            #[serde(flatten)]
            specific: Props,
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Fields {
            String(String),
            Props(SerdeProps),
        }

        let (uri, fields) = BTreeMap::<SourceUri, Fields>::deserialize(deserializer)?
            .into_iter()
            .next()
            .ok_or(serde::de::Error::custom("no upstream"))?;
        let (unpack_dir, props) = match fields {
            Fields::String(hash) => match &uri.kind {
                Kind::Archive => (None, Props::default_plain(hash)),
                Kind::Git => (None, Props::default_git(hash)),
            },
            Fields::Props(props) => match (&props.specific, &uri.kind) {
                (Props::Git { .. }, Kind::Archive) | (Props::Plain { .. }, Kind::Git) => {
                    return Err(serde::de::Error::custom("mismatched URL type and upstream properties"));
                }
                _ => (props.destdir, props.specific),
            },
        };

        Ok(Self {
            url: uri.into(),
            unpack_dir,
            props,
        })
    }
}

/// Supported kinds of upstream in a recipe.
#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub enum Kind {
    /// The upstream is an archive, typically a tarball.
    Archive,
    /// The upstream is a git repository.
    Git,
}

/// A URI from where to download source code.
/// The URI is a combination of a URL plus the kind of
/// upstream, that instructs downloaders how to fetch
/// the resource.
///
/// ### String representation
///
/// In the case of an archive, a regular URL is used.
///
/// For a git repository, the URI is parsed and unparsed
/// with the format `git|<regular_url>`.
#[derive(Clone, Debug, Deserialize, Eq, PartialOrd, Ord, PartialEq)]
#[serde(try_from = "&str")]
pub struct SourceUri {
    /// The kind of source the URL refers to.
    pub kind: Kind,
    /// Location of the source.
    pub url: Url,
}

impl FromStr for SourceUri {
    type Err = url::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(git_url) = s.strip_prefix(GIT_PREFIX) {
            Ok(SourceUri {
                kind: Kind::Git,
                url: git_url.parse()?,
            })
        } else {
            Ok(SourceUri {
                kind: Kind::Archive,
                url: s.parse()?,
            })
        }
    }
}

impl TryFrom<&str> for SourceUri {
    type Error = url::ParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl Display for SourceUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            Kind::Archive => write!(f, "{}", self.url.as_str()),
            Kind::Git => {
                write!(f, "{GIT_PREFIX}{}", self.url.as_str())
            }
        }
    }
}

impl From<SourceUri> for Url {
    fn from(value: SourceUri) -> Self {
        value.url
    }
}

impl Borrow<Url> for SourceUri {
    fn borrow(&self) -> &Url {
        &self.url
    }
}

#[derive(Clone, Deserialize, Debug)]
#[serde(untagged)]
pub enum Props {
    Plain {
        hash: String,
        rename: Option<String>,
        #[serde(rename = "stripdirs")]
        strip_dirs: Option<u8>,
        #[serde(default = "default_true", deserialize_with = "stringy_bool")]
        unpack: bool,
    },
    Git {
        #[serde(rename = "ref")]
        git_ref: String,
        #[serde(default = "default_true", deserialize_with = "stringy_bool")]
        staging: bool,
    },
}

impl Props {
    fn default_plain(hash: String) -> Self {
        Self::Plain {
            hash,
            rename: None,
            strip_dirs: None,
            unpack: true,
        }
    }

    fn default_git(git_ref: String) -> Self {
        Self::Git { git_ref, staging: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SRC_URL: &str = "https://example.com/source";

    #[test]
    fn parse_archive() -> Result<(), url::ParseError> {
        let src: SourceUri = SRC_URL.parse()?;
        assert_eq!(
            src,
            SourceUri {
                kind: Kind::Archive,
                url: Url::from_str(SRC_URL)?
            }
        );
        Ok(())
    }

    #[test]
    fn parse_git() -> Result<(), url::ParseError> {
        let src: SourceUri = format!("{GIT_PREFIX}{SRC_URL}").parse()?;
        assert_eq!(
            src,
            SourceUri {
                kind: Kind::Git,
                url: Url::from_str(SRC_URL)?
            }
        );
        Ok(())
    }
}
