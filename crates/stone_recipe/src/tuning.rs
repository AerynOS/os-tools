// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

use crate::{KeyValue, Macros, sequence_of_key_value, single_as_sequence};

#[derive(Debug, Clone)]
pub enum Tuning {
    Enable,
    Disable,
    Config(String),
}

impl<'de> Deserialize<'de> for KeyValue<Tuning> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Inner {
            Bool(bool),
            Config(String),
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum Outer {
            Key(String),
            KeyValue(BTreeMap<String, Inner>),
        }

        match Outer::deserialize(deserializer)? {
            Outer::Key(key) => Ok(KeyValue {
                key,
                value: Tuning::Enable,
            }),
            Outer::KeyValue(map) => match map.into_iter().next() {
                Some((key, Inner::Bool(true))) => Ok(KeyValue {
                    key,
                    value: Tuning::Enable,
                }),
                Some((key, Inner::Bool(false))) => Ok(KeyValue {
                    key,
                    value: Tuning::Disable,
                }),
                Some((key, Inner::Config(config))) => Ok(KeyValue {
                    key,
                    value: Tuning::Config(config),
                }),
                // unreachable?
                None => Err(serde::de::Error::custom("missing tuning entry")),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuningFlag {
    #[serde(flatten)]
    root: CompilerFlags,
    #[serde(default)]
    gnu: CompilerFlags,
    #[serde(default)]
    llvm: CompilerFlags,
}

impl TuningFlag {
    pub fn get(&self, flag: CompilerFlag, toolchain: Toolchain) -> Option<&str> {
        match toolchain {
            Toolchain::Llvm => self.llvm.get(flag),
            Toolchain::Gnu => self.gnu.get(flag),
        }
        .or_else(|| self.root.get(flag))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CompilerFlag {
    C,
    Cxx,
    F,
    D,
    Rust,
    Ld,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CompilerFlags {
    c: Option<String>,
    cxx: Option<String>,
    f: Option<String>,
    d: Option<String>,
    rust: Option<String>,
    ld: Option<String>,
}

impl CompilerFlags {
    fn get(&self, flag: CompilerFlag) -> Option<&str> {
        match flag {
            CompilerFlag::C => self.c.as_deref(),
            CompilerFlag::Cxx => self.cxx.as_deref(),
            CompilerFlag::F => self.f.as_deref(),
            CompilerFlag::D => self.d.as_deref(),
            CompilerFlag::Rust => self.rust.as_deref(),
            CompilerFlag::Ld => self.ld.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Toolchain {
    #[default]
    Llvm,
    Gnu,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuningOption {
    #[serde(default, deserialize_with = "single_as_sequence")]
    pub enabled: Vec<String>,
    #[serde(default, deserialize_with = "single_as_sequence")]
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuningGroup {
    #[serde(flatten, default)]
    pub root: TuningOption,
    pub default: Option<String>,
    #[serde(default, rename = "options", deserialize_with = "sequence_of_key_value")]
    pub choices: Vec<KeyValue<TuningOption>>,
}

#[derive(Debug, Default)]
pub struct Builder {
    flags: BTreeMap<String, TuningFlag>,
    groups: BTreeMap<String, TuningGroup>,
    enabled: BTreeSet<String>,
    disabled: BTreeSet<String>,
    option_sets: BTreeMap<String, String>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_flag(&mut self, name: impl ToString, flag: TuningFlag) {
        self.flags.insert(name.to_string(), flag);
    }

    pub fn add_group(&mut self, name: impl ToString, group: TuningGroup) {
        self.groups.insert(name.to_string(), group);
    }

    pub fn add_macros(&mut self, macros: Macros) {
        for kv in macros.flags {
            self.add_flag(kv.key, kv.value);
        }
        for kv in macros.tuning {
            self.add_group(kv.key, kv.value);
        }
    }

    pub fn enable(&mut self, name: impl ToString, config: Option<String>) -> Result<(), Error> {
        let name = name.to_string();

        let group = self
            .groups
            .get(&name)
            .ok_or_else(|| Error::UnknownGroup(name.clone()))?;

        self.enabled.insert(name.clone());
        self.disabled.remove(&name);

        if let Some(value) = config.or_else(|| group.default.clone()) {
            if group.choices.iter().any(|kv| kv.key == value) {
                self.option_sets.insert(name, value);
            } else {
                return Err(Error::UnknownGroupValue(value, name));
            }
        }

        Ok(())
    }

    pub fn disable(&mut self, name: impl ToString) -> Result<(), Error> {
        let name = name.to_string();

        if !self.groups.contains_key(&name) {
            return Err(Error::UnknownGroup(name));
        }

        self.disabled.insert(name.clone());
        self.enabled.remove(&name);
        self.option_sets.remove(&name);

        Ok(())
    }

    pub fn build(&self) -> Result<Vec<TuningFlag>, Error> {
        let mut enabled_flags = BTreeSet::new();
        let mut disabled_flags = BTreeSet::new();

        for enabled in &self.enabled {
            let Some(group) = self.groups.get(enabled) else {
                continue;
            };

            let mut to = &group.root;

            if let Some(option) = self.option_sets.get(enabled) {
                if let Some(choice) = group.choices.iter().find(|kv| &kv.key == option) {
                    to = &choice.value;
                }
            }

            enabled_flags.extend(to.enabled.clone());
        }

        for disabled in &self.disabled {
            let Some(group) = self.groups.get(disabled) else {
                continue;
            };
            disabled_flags.extend(group.root.disabled.clone());
        }

        for flag in enabled_flags.iter().chain(&disabled_flags) {
            if !self.flags.contains_key(flag) {
                return Err(Error::UnknownFlag(flag.clone()));
            }
        }

        Ok(enabled_flags
            .iter()
            .chain(&disabled_flags)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|flag| self.flags.get(flag).cloned())
            .collect())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown flag {0}")]
    UnknownFlag(String),
    #[error("unknown group {0}")]
    UnknownGroup(String),
    #[error("unknown value {0} for group {1}")]
    UnknownGroupValue(String, String),
}
