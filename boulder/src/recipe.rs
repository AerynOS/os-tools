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
use stone_recipe::control_file;
use thiserror::Error;
use tui::Styled;

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
        let control_file_path = path.with_file_name("control.kdl");

        let source = fs::read_to_string(&path).map_err(Error::LoadRecipe)?;
        let mut parsed = stone_recipe::from_str(&source)?;

        // Apply control file if it exists
        if control_file_path.exists() {
            let content = fs::read_to_string(&control_file_path).map_err(Error::LoadControlFile)?;
            let control_file = control_file::decode(&content)
                .map_err(|err| Error::DecodeControlFile(err, control_file_path.clone()))?;

            control_file
                .apply_to_recipe(&mut parsed)
                .map_err(|err| Error::ApplyControlFile(err, control_file_path.clone()))?;

            println!(
                "{} | Applied modifications from {control_file_path:?}",
                "Control File".green()
            );
        }

        let build_time = resolve_build_time(&path);

        // Invariant checks

        // We want versions to start with an integer for ent comparison purposes
        if !parsed.source.version.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(Error::Value(format!(
                "version must start with an integer (found 'version: {}')",
                parsed.source.version
            )));
        }

        // Setting release to 0 is a common mistake
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
    if let Ok(epoch_env) = env::var("SOURCE_DATE_EPOCH")
        && let Ok(parsed) = epoch_env.parse::<i64>()
        && let Some(timestamp) = DateTime::from_timestamp(parsed, 0)
    {
        return timestamp;
    }

    // If we are building from a git repo and have the git binary available to us then use the last commit timestamp
    if let Some(recipe_dir) = path.parent()
        && let Ok(git_log) = Command::new("git")
            .args(["log", "-1", "--format=\"%at\""])
            .current_dir(recipe_dir)
            .output()
        && let Ok(stdout) = String::from_utf8(git_log.stdout)
        && let Ok(parsed) = stdout.replace(['\n', '"'], "").parse::<i64>()
        && let Some(timestamp) = DateTime::from_timestamp(parsed, 0)
    {
        return timestamp;
    }

    // As a final fallback use the current time
    Utc::now()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("recipe file does not exist: {0:?}")]
    MissingRecipe(PathBuf),
    #[error("load recipe")]
    LoadRecipe(#[source] io::Error),
    #[error("load control file")]
    LoadControlFile(#[source] io::Error),
    #[error("decode recipe")]
    Decode(#[from] stone_recipe::Error),
    #[error("value: {0}")]
    Value(String),
    #[error("failed to decode control file {1:?}")]
    DecodeControlFile(#[source] control_file::decode::Error, PathBuf),
    #[error("failed to modify recipe with control file {1:?}")]
    ApplyControlFile(#[source] control_file::ModificationError, PathBuf),
}
