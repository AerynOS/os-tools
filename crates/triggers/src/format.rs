// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

// [`Pattern`] has Regex inside which has interior mutability,
// but we don't Ord or Hash off that field
#![allow(clippy::mutable_key_type)]

use std::collections::BTreeMap;

use fnmatch::Pattern;
use serde::Deserialize;

/// Filter matched paths to a specific kind
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathKind {
    Directory,
    Symlink,
}

/// Execution handlers for a trigger
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum Handler {
    Run { run: String, args: Vec<String> },
    Delete { delete: Vec<String> },
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompiledHandler(Handler);

impl CompiledHandler {
    pub fn handler(&self) -> &Handler {
        &self.0
    }
}

impl Handler {
    /// Substitute all paths using matched variables
    pub fn compiled(&self, with_match: &fnmatch::Match) -> CompiledHandler {
        match self {
            Handler::Run { run, args } => {
                let mut run = run.clone();
                for (key, value) in &with_match.variables {
                    run = run.replace(&format!("$({key})"), value);
                }
                let args = args
                    .iter()
                    .map(|a| {
                        let mut a = a.clone();
                        for (key, value) in &with_match.variables {
                            a = a.replace(&format!("$({key})"), value);
                        }
                        a
                    })
                    .collect();
                CompiledHandler(Handler::Run { run, args })
            }
            Handler::Delete { delete } => CompiledHandler(Handler::Delete { delete: delete.clone() }),
        }
    }
}

/// Inhibitors prevent handlers from running based on some constraints
#[derive(Debug, Deserialize)]
pub struct Inhibitors {
    pub paths: Vec<String>,
    pub environment: Vec<String>,
}

/// Map handlers to a path pattern and kind filter
#[derive(Debug, Deserialize)]
pub struct PathDefinition {
    pub handlers: Vec<String>,
    #[serde(rename = "type")]
    pub kind: Option<PathKind>,
}

/// Serialization format of triggers
#[derive(Debug, Deserialize)]
pub struct Trigger {
    /// Unique (global scope) identifier
    pub name: String,

    /// User friendly description
    pub description: String,

    /// Run before this trigger name
    pub before: Option<String>,

    /// Run after this trigger name
    pub after: Option<String>,

    /// Optional inhibitors
    pub inhibitors: Option<Inhibitors>,

    /// Map glob / patterns to their configuration
    pub paths: BTreeMap<Pattern, PathDefinition>,

    /// Named handlers within this trigger scope
    pub handlers: BTreeMap<String, Handler>,
}

#[cfg(test)]
mod tests {
    use crate::format::Trigger;

    #[test]
    fn test_trigger_file() {
        let trigger: Trigger = serde_yaml::from_str(include_str!("../../../test/trigger.yml")).unwrap();

        let (pattern, _) = trigger.paths.iter().next().expect("Missing path entry");
        let result = pattern
            .match_path("/usr/lib/modules/6.6.7-267.current/kernel")
            .expect("Couldn't match path");
        let version = result.variables.get("version").expect("Missing kernel version");
        assert_eq!(version, "6.6.7-267.current", "Wrong kernel version match");
        eprintln!("trigger: {trigger:?}");
        eprintln!("match: {result:?}");
    }
}
