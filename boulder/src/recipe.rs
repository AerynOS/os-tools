// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    env, io,
    path::{Path, PathBuf},
    process::Command,
};

use chrono::{DateTime, Utc};
use fs_err as fs;
use thiserror::Error;

use crate::architecture::{self, BuildTarget};

pub type Parsed = stone_recipe::Recipe;

#[derive(Debug)]
pub struct Recipe {
    pub path: PathBuf,
    pub source: String,
    pub parsed: Parsed,
    pub build_time: DateTime<Utc>,
}

impl Recipe {
    /// Desired recipe value invariants are checked here
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = resolve_path(path)?;
        let source = fs::read_to_string(&path)?;
        let parsed = stone_recipe::from_str(&source)?;
        let build_time = resolve_build_time(&path);

        // Invariant checks
        if parsed.source.release == 0 {
            return Err(Error::Value(format!(
                "release must be > 0 (found 'release: {}')",
                parsed.source.release
            )));
        }
        // Invariant checks done

        Ok(Self {
            path,
            source,
            parsed,
            build_time,
        })
    }

    pub fn build_targets(&self) -> Vec<BuildTarget> {
        let host = architecture::host();
        let host_string = host.to_string();

        let mut targets = vec![];
        if self.parsed.architectures.is_empty() {
            if self.parsed.emul32 {
                targets.push(BuildTarget::Emul32(host));
            }

            targets.push(BuildTarget::Native(host));
        } else {
            let emul32 = BuildTarget::Emul32(host);
            let emul32_string = emul32.to_string();

            if self.parsed.architectures.contains(&emul32_string)
                || self.parsed.architectures.contains(&"emul32".into())
            {
                targets.push(emul32);
            }

            if self.parsed.architectures.contains(&host_string) || self.parsed.architectures.contains(&"native".into())
            {
                targets.push(BuildTarget::Native(host));
            }
        }

        targets
    }

    pub fn build_target_profile_key(&self, target: BuildTarget) -> Option<String> {
        let target_string = target.to_string();

        if self.parsed.profiles.iter().any(|kv| kv.key == target_string) {
            Some(target_string)
        } else if target.emul32() && self.parsed.profiles.iter().any(|kv| &kv.key == "emul32") {
            Some("emul32".to_owned())
        } else {
            None
        }
    }

    pub fn build_target_definition(&self, target: BuildTarget) -> &stone_recipe::Build {
        let key = self.build_target_profile_key(target);

        if let Some(profile) = self.parsed.profiles.iter().find(|kv| Some(&kv.key) == key.as_ref()) {
            &profile.value
        } else {
            &self.parsed.build
        }
    }
}

pub fn resolve_path(path: impl AsRef<Path>) -> Result<PathBuf, Error> {
    let path = path.as_ref();

    // Resolve dir to dir + stone.yaml
    let path = if path.is_dir() {
        path.join("stone.yaml")
    } else {
        path.to_path_buf()
    };

    // Ensure it's absolute & exists
    fs::canonicalize(&path).map_err(|_| Error::MissingRecipe(path))
}

fn resolve_build_time(path: &Path) -> DateTime<Utc> {
    // Propagate SOURCE_DATE_EPOCH if set
    if let Ok(epoch_env) = env::var("SOURCE_DATE_EPOCH") {
        if let Ok(parsed) = epoch_env.parse::<i64>() {
            if let Some(timestamp) = DateTime::from_timestamp(parsed, 0) {
                return timestamp;
            }
        }
    }

    // If we are building from a git repo and have the git binary available to us then use the last commit timestamp
    if let Some(recipe_dir) = path.parent() {
        if let Ok(git_log) = Command::new("git")
            .args(["log", "-1", "--format=\"%at\""])
            .current_dir(recipe_dir)
            .output()
        {
            if let Ok(stdout) = String::from_utf8(git_log.stdout) {
                if let Ok(parsed) = stdout.replace(['\n', '"'], "").parse::<i64>() {
                    if let Some(timestamp) = DateTime::from_timestamp(parsed, 0) {
                        return timestamp;
                    }
                }
            }
        }
    }

    // As a final fallback use the current time
    Utc::now()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("recipe file does not exist: {0:?}")]
    MissingRecipe(PathBuf),
    #[error("load recipe")]
    Load(#[from] io::Error),
    #[error("decode recipe")]
    Decode(#[from] stone_recipe::Error),
    #[error("value: {0}")]
    Value(String),
}
