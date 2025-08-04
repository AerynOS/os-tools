// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::{hash::Hash, path::PathBuf};

use serde::Deserialize;
pub use serde_yaml::Error;
use thiserror::Error;
use url::Url;

pub use self::macros::Macros;
pub use self::script::Script;
pub use self::tuning::Tuning;

pub mod macros;
pub mod script;
pub mod tuning;

fn is_valid_commit_hash(s: &str) -> bool {
    // git commit hashes can be SHA-1 or SHA-256 hashes
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn from_slice(bytes: &[u8]) -> Result<Recipe, Error> {
    serde_yaml::from_slice(bytes)
}

pub fn from_str(s: &str) -> Result<Recipe, Error> {
    serde_yaml::from_str(s)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    #[serde(flatten)]
    pub source: Source,
    #[serde(flatten)]
    pub build: Build,
    #[serde(flatten)]
    pub package: Package,
    #[serde(flatten)]
    pub options: Options,
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub profiles: Vec<KeyValue<Build>>,
    #[serde(default, rename = "packages", deserialize_with = "sequence_of_key_value")]
    pub sub_packages: Vec<KeyValue<Package>>,
    #[serde(default)]
    pub upstreams: Vec<Upstream>,
    #[serde(default)]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub tuning: Vec<KeyValue<Tuning>>,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub emul32: bool,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub mold: bool,
}

#[derive(Debug, Clone)]
pub struct KeyValue<T> {
    pub key: String,
    pub value: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub name: String,
    #[serde(deserialize_with = "force_string")]
    pub version: String,
    pub release: u64,
    pub homepage: String,
    #[serde(deserialize_with = "single_as_sequence")]
    pub license: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Build {
    pub setup: Option<String>,
    pub build: Option<String>,
    pub install: Option<String>,
    pub check: Option<String>,
    pub workload: Option<String>,
    pub environment: Option<String>,
    #[serde(default, rename = "builddeps")]
    pub build_deps: Vec<String>,
    #[serde(default, rename = "checkdeps")]
    pub check_deps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub toolchain: tuning::Toolchain,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub cspgo: bool,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub samplepgo: bool,
    #[serde(default = "default_true", deserialize_with = "stringy_bool")]
    pub debug: bool,
    #[serde(default = "default_true", deserialize_with = "stringy_bool")]
    pub strip: bool,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub networking: bool,
    #[serde(default, deserialize_with = "stringy_bool")]
    pub compressman: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(default, rename = "rundeps")]
    pub run_deps: Vec<String>,
    #[serde(default)]
    pub paths: Vec<Path>,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Upstream {
    Plain {
        uri: Url,
        hash: String,
        rename: Option<String>,
        strip_dirs: Option<u8>,
        unpack: bool,
        unpack_dir: Option<PathBuf>,
    },
    Git {
        uri: Url,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
        clone_dir: Option<PathBuf>,
        staging: bool,
    },
}

impl<'de> Deserialize<'de> for Upstream {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Inner {
            Plain {
                hash: String,
                rename: Option<String>,
                #[serde(rename = "stripdirs")]
                strip_dirs: Option<u8>,
                #[serde(default = "default_true", deserialize_with = "stringy_bool")]
                unpack: bool,
                #[serde(rename = "unpackdir")]
                unpack_dir: Option<PathBuf>,
            },
            Git {
                tag: Option<String>,
                branch: Option<String>,
                rev: Option<String>,
                #[serde(rename = "clonedir")]
                clone_dir: Option<PathBuf>,
                #[serde(default = "default_true", deserialize_with = "stringy_bool")]
                staging: bool,
            },
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Outer {
            String(String),
            Inner(Inner),
        }

        #[derive(Debug, Deserialize, PartialEq, Eq, Ord, PartialOrd, Hash)]
        #[serde(try_from = "&str")]
        enum Uri {
            Plain(Url),
            Git(Url),
        }

        impl<'a> TryFrom<&'a str> for Uri {
            type Error = UriParseError;

            fn try_from(s: &'a str) -> Result<Self, Self::Error> {
                match s.split_once("git|") {
                    Some((_, uri)) => Ok(Uri::Git(uri.parse()?)),
                    None => Ok(Uri::Plain(s.parse()?)),
                }
            }
        }

        #[derive(Debug, Error)]
        #[error("invalid uri: {0}")]
        struct UriParseError(#[from] url::ParseError);

        // Helper function to validate the rev for git type upstreams to ensure it is a valid commit hash.
        fn validate_rev<'de, D>(rev: &str) -> Result<(), D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            if !is_valid_commit_hash(rev) {
                return Err(serde::de::Error::custom(format!(
                    "'{rev}' is not a valid git commit hash. If this is a tag or branch name use the corresponding 'tag' or 'branch' keys instead. For example: `tag: {rev}`"
                )));
            }
            Ok(())
        }

        let raw_map = BTreeMap::<Uri, Outer>::deserialize(deserializer)?;

        match raw_map.into_iter().next() {
            Some((Uri::Plain(uri), Outer::String(hash))) => Ok(Upstream::Plain {
                uri,
                hash,
                rename: None,
                strip_dirs: None,
                unpack: default_true(),
                unpack_dir: None,
            }),
            Some((Uri::Git(uri), Outer::String(rev))) => {
                validate_rev::<D>(&rev)?;
                Ok(Upstream::Git {
                    uri,
                    tag: None,
                    branch: None,
                    rev: Some(rev),
                    clone_dir: None,
                    staging: default_true(),
                })
            }
            Some((
                Uri::Plain(uri),
                Outer::Inner(Inner::Plain {
                    hash,
                    rename,
                    strip_dirs,
                    unpack,
                    unpack_dir,
                }),
            )) => Ok(Upstream::Plain {
                uri,
                hash,
                rename,
                strip_dirs,
                unpack,
                unpack_dir,
            }),
            Some((
                Uri::Git(uri),
                Outer::Inner(Inner::Git {
                    tag,
                    branch,
                    rev,
                    clone_dir,
                    staging,
                }),
            )) => {
                // We prefer using rev, but allow tags and branches
                // Regardless of which source is given here, the corresponding commit hash
                // will be fetched and stored in the stone.lock file for reproducibility
                let git_source_count = tag.is_some() as u32 + branch.is_some() as u32 + rev.is_some() as u32;
                if git_source_count != 1 {
                    return Err(serde::de::Error::custom(
                        "For a git upstream, you must specify exactly one of: tag, branch, or rev",
                    ));
                }
                if let Some(rev_string) = &rev {
                    validate_rev::<D>(rev_string)?;
                }
                Ok(Upstream::Git {
                    uri,
                    tag,
                    branch,
                    rev,
                    clone_dir,
                    staging,
                })
            }
            Some((Uri::Plain(_), Outer::Inner(Inner::Git { .. }))) => Err(serde::de::Error::custom(
                "found git payload but missing 'git|' prefixed URI",
            )),
            Some((Uri::Git(_), Outer::Inner(Inner::Plain { .. }))) => {
                Err(serde::de::Error::custom("found git URI but plain payload fields"))
            }
            // unreachable?
            None => Err(serde::de::Error::custom("missing upstream entry")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub path: String,
    pub kind: PathKind,
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Inner {
            String(String),
            KeyValue(BTreeMap<String, PathKind>),
        }

        match Inner::deserialize(deserializer)? {
            Inner::String(path) => Ok(Path {
                path,
                kind: PathKind::default(),
            }),
            Inner::KeyValue(map) => {
                if let Some((path, kind)) = map.into_iter().next() {
                    Ok(Path { path, kind })
                } else {
                    Err(serde::de::Error::custom("missing path entry"))
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::EnumString, Default)]
#[serde(try_from = "&str")]
#[strum(serialize_all = "lowercase")]
pub enum PathKind {
    #[default]
    Any,
    Exe,
    Symlink,
    Special,
}

fn default_true() -> bool {
    true
}

/// Deserialize a single value or sequence of values as a vec
fn single_as_sequence<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum Value<T> {
        Single(T),
        Sequence(Vec<T>),
    }

    match Value::deserialize(deserializer)? {
        Value::Single(value) => Ok(vec![value]),
        Value::Sequence(sequence) => Ok(sequence),
    }
}

/// Deserialize a sequence of single entry maps as a vec of [`KeyValue`]
fn sequence_of_key_value<'de, T, D>(deserializer: D) -> Result<Vec<KeyValue<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let sequence = Vec::<BTreeMap<String, T>>::deserialize(deserializer)?;

    Ok(sequence.into_iter().fold(vec![], |acc, next| {
        acc.into_iter()
            .chain(next.into_iter().next().map(|(key, value)| KeyValue { key, value }))
            .collect()
    }))
}

fn stringy_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Inner {
        Bool(bool),
        String(String),
    }

    match Inner::deserialize(deserializer)? {
        Inner::Bool(bool) => Ok(bool),
        // Really yaml...
        Inner::String(s) => match s.as_str() {
            "y" | "Y" | "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => Ok(true),
            "n" | "N" | "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => Ok(false),
            _ => Err(serde::de::Error::custom("invalid boolean: expected true or false")),
        },
    }
}

fn force_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Inner {
        String(String),
        Number(serde_yaml::Number),
    }

    match Inner::deserialize(deserializer)? {
        Inner::String(s) => Ok(s),
        Inner::Number(n) => Ok(n.to_string()),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn deserialize() {
        let inputs = [
            &include_bytes!("../../../test/llvm-stone.yml")[..],
            &include_bytes!("../../../test/boulder-stone.yml")[..],
        ];

        for input in inputs {
            let recipe = from_slice(input).unwrap();
            dbg!(&recipe);
        }
    }

    #[test]
    fn reject_git_tag() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo : v1.0.0
"#;
        let result = from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn accept_git_tag() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        tag: v1.0.0
"#;
        let result = from_str(input);
        assert!(result.is_ok());
    }

    #[test]
    fn accept_git_branch() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        branch: main
"#;
        let result = from_str(input);
        assert!(result.is_ok());
    }

    #[test]
    fn accept_git_rev() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        rev: a94a8fe5ccb19ba61c4c0873d391e987982fbbd3
"#;
        let result = from_str(input);
        assert!(result.is_ok());
    }

    #[test]
    fn reject_git_rev() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        rev: v1.0.0
"#;
        let result = from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn reject_git_rev_with_tag() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        tag: v1.0.0
        rev: a94a8fe5ccb19ba61c4c0873d391e987982fbbd3
"#;
        let result = from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn reject_git_rev_with_branch() {
        let input = r#"
name: test-pkg
version: 1.0.0
release: 1
license: MIT
homepage: https://example.com
upstreams:
    - git|https://github.com/example/repo:
        branch: main
        rev: a94a8fe5ccb19ba61c4c0873d391e987982fbbd3
"#;
        let result = from_str(input);
        assert!(result.is_err());
    }
}
