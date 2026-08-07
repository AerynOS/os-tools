// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

// [`Pattern`] has Regex inside which has interior mutability,
// but we don't Ord or Hash off that field
#![allow(clippy::mutable_key_type)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::Error;
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
///
/// `run` arguments support variable expansion with explicit modes:
///
/// * `$(var)` / `$(var:each)` - one invocation per match value
/// * `$(var:all)` - Singular invocation for every match's value expanded inline
///
/// Bare `$(var)` arguments default to the per-match mode, preserving the
/// pre-coalescing behaviour for existing triggers.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum Handler {
    Run { run: String, args: Vec<String> },
    Delete { delete: Vec<String> },
}

impl fmt::Display for Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = match self {
            Handler::Run { run, args } => {
                f.write_str(run)?;
                args
            }
            Handler::Delete { delete } => {
                f.write_str("rm --")?;
                delete
            }
        };

        // Note: No shell quoting for simplicity.
        // Could use the shell-quote crate if we wanted to be more correct.
        for arg in args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompiledHandler(Handler);

impl CompiledHandler {
    pub fn handler(&self) -> &Handler {
        &self.0
    }
}

/// Expansion mode for a variable argument
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VarMode {
    /// One invocation per distinct match value
    Each,

    /// Expand every match's value inline into a single invocation
    All,
}

/// A parsed trigger argument
#[derive(Debug, Clone)]
enum ArgKind {
    /// Static argument
    Fixed(String),

    /// $(var) / $(var:each) / $(var:all) arguments
    Variable { template: String, mode: VarMode },
}

/// Substitute every $(var), $(var:each) and $(var:all) reference in template
fn substitute(template: &str, variables: &BTreeMap<String, String>) -> String {
    let mut result = template.to_owned();
    for (key, value) in variables {
        result = result.replace(&format!("$({key})"), value);
        result = result.replace(&format!("$({key}:each)"), value);
        result = result.replace(&format!("$({key}:all)"), value);
    }
    result
}

/// Parse an argument into its fixed or variable form
fn parse_arg(arg: &str) -> Result<ArgKind, Error> {
    let Some(inner) = arg.strip_prefix("$(").and_then(|s| s.strip_suffix(')')) else {
        return Ok(ArgKind::Fixed(arg.to_owned()));
    };

    let mode = match inner.split_once(':') {
        Some((_, "each")) => VarMode::Each,
        Some((_, "all")) => VarMode::All,
        Some((_, other)) => return Err(Error::UnknownArgMode(other.to_owned())),
        // Bare $(var) behaves like $(var:each) for backwards compatibility
        None => VarMode::Each,
    };

    Ok(ArgKind::Variable {
        template: arg.to_owned(),
        mode,
    })
}

impl Handler {
    /// Coalesce multiple matches into as few handler executions as possible
    ///
    /// * $(var) / $(var:each) - one invocation per distinct match value
    /// * $(var:all) - a single invocation with every match's value expanded inline
    ///
    /// Fixed (non-variable) arguments are also substituted with the match variables,
    /// and act as grouping keys when they reference variables, e.g. an argument of
    /// --version=$(version) results in one invocation per distinct version.
    pub fn coalesced(&self, matches: &BTreeSet<fnmatch::Match>) -> Result<Vec<CompiledHandler>, Error> {
        match self {
            Handler::Run { run, args } => {
                if matches.is_empty() {
                    return Ok(vec![]);
                }

                let arg_kinds = args.iter().map(|a| parse_arg(a)).collect::<Result<Vec<_>, _>>()?;

                // Any ":each" argument means one invocation per match, with every
                // argument substituted using that match's variables
                let has_each = arg_kinds.iter().any(|k| {
                    matches!(
                        k,
                        ArgKind::Variable {
                            mode: VarMode::Each,
                            ..
                        }
                    )
                });

                if has_each {
                    // Deduplicate identical commands: many matched paths can share
                    // the same captured variables (e.g. every module path for one
                    // kernel version), which must not re-run the handler per path
                    return Ok(matches
                        .iter()
                        .map(|m| {
                            CompiledHandler(Handler::Run {
                                run: substitute(run, &m.variables),
                                args: arg_kinds
                                    .iter()
                                    .map(|kind| match kind {
                                        ArgKind::Fixed(template) => substitute(template, &m.variables),
                                        ArgKind::Variable { template, .. } => substitute(template, &m.variables),
                                    })
                                    .collect(),
                            })
                        })
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect());
                }

                // All variable arguments are ":all": group matches by the fixed shape
                // of the command (substituted run + fixed args), then expand the ":all"
                // arguments per match within each group, preserving value pairing
                let mut groups: BTreeMap<(String, Vec<String>), Vec<&fnmatch::Match>> = BTreeMap::new();
                for m in matches {
                    let key = (
                        substitute(run, &m.variables),
                        arg_kinds
                            .iter()
                            .filter_map(|kind| match kind {
                                ArgKind::Fixed(template) => Some(substitute(template, &m.variables)),
                                ArgKind::Variable { .. } => None,
                            })
                            .collect(),
                    );
                    groups.entry(key).or_default().push(m);
                }

                let mut results = Vec::new();
                for ((final_run, fixed_args), group_matches) in groups {
                    // Fixed args are identical for every match in the group
                    let mut final_args = fixed_args.clone();

                    // Expand the ":all" arguments per match, preserving the
                    // pairing of variables captured from the same path, and
                    // deduplicating matches that share identical variables
                    let mut seen = BTreeSet::new();
                    for m in &group_matches {
                        if seen.insert(&m.variables) {
                            for kind in &arg_kinds {
                                if let ArgKind::Variable { template, .. } = kind {
                                    final_args.push(substitute(template, &m.variables));
                                }
                            }
                        }
                    }
                    results.push(CompiledHandler(Handler::Run {
                        run: final_run,
                        args: final_args,
                    }));
                }
                Ok(results)
            }

            Handler::Delete { delete } => Ok(vec![CompiledHandler(Handler::Delete { delete: delete.clone() })]),
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
