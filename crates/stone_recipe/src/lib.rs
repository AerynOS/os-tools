// SPDX-FileCopyrightText: Copyright © 2020-2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
pub use serde_yaml::Error;
use thiserror::Error;
use url::Url;

use crate::upstream::{Kind, SourceUri};

pub use self::macros::Macros;
pub use self::script::Script;
pub use self::tuning::Tuning;

pub mod macros;
pub mod script;
pub mod tuning;
pub mod upstream;

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
    #[serde(default = "default_true", deserialize_with = "stringy_bool")]
    pub lastrip: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(default, rename = "rundeps")]
    pub run_deps: Vec<String>,
    #[serde(default, rename = "rundeps-exclude")]
    pub run_deps_exclude: Vec<String>,
    #[serde(default)]
    pub paths: Vec<Path>,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(untagged)]
pub enum UpstreamProps {
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

impl UpstreamProps {
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

#[derive(Debug, Clone)]
pub struct Upstream {
    pub url: Url,
    pub unpack_dir: Option<PathBuf>,
    pub props: UpstreamProps,
}

impl<'de> Deserialize<'de> for Upstream {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        struct Props {
            destdir: Option<PathBuf>,
            #[serde(flatten)]
            specific: UpstreamProps,
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Fields {
            String(String),
            Props(Props),
        }

        let (uri, fields) = BTreeMap::<SourceUri, Fields>::deserialize(deserializer)?
            .into_iter()
            .next()
            .ok_or(serde::de::Error::custom("no upstream"))?;
        let (unpack_dir, props) = match fields {
            Fields::String(hash) => match &uri.kind {
                Kind::Archive => (None, UpstreamProps::default_plain(hash)),
                Kind::Git => (None, UpstreamProps::default_git(hash)),
            },
            Fields::Props(props) => match (&props.specific, &uri.kind) {
                (UpstreamProps::Git { .. }, Kind::Archive) | (UpstreamProps::Plain { .. }, Kind::Git) => {
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
}
