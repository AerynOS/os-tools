// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::Path, process, string};

use crate::Recipe;
use fs_err as fs;
use thiserror::Error;
use tui::Styled;
use url::Url;
use yaml;

fn is_valid_commit_hash(s: &str) -> bool {
    // git commit hashes can be SHA-1 or SHA-256 hashes
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolves a git reference to its commit hash using `git ls-remote`.
pub fn resolve_git_ref(uri: &Url, ref_id: &str) -> Result<String, GitError> {
    let refs_to_try = [format!("refs/tags/{ref_id}"), format!("refs/heads/{ref_id}")];
    let output = process::Command::new("git")
        .args(["ls-remote", "--", uri.as_str()])
        .args(&refs_to_try)
        .output()?;

    if !output.status.success() {
        return Err(GitError::LsRemoteFailed(uri.clone()));
    }
    let stdout = String::from_utf8(output.stdout)?;

    // git ls-remote output is in the format: <hash>\t<ref_name>
    // so just grab the first word to get the hash.
    stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| GitError::UnresolvedRef {
            ref_id: ref_id.to_owned(),
            uri: uri.clone(),
        })
        .map(|s| s.to_owned())
}

/// Replaces the non-hash refs for git upstreams with the hash for the given ref
/// and includes a comment showing the original ref.
pub fn update_git_upstream_ref_in_yaml(
    updater: &mut yaml::Updater,
    upstream_index: usize,
    uri: &str,
    new_ref: &str,
    original_ref: &str,
) {
    let git_key = format!("git|{uri}");
    let new_value_with_comment = format!("{new_ref} # {original_ref}");

    // git|uri: <ref>
    updater.update_value(&new_value_with_comment, |p| {
        p / "upstreams" / upstream_index / git_key.as_str()
    });

    // git|uri:
    // - ref: <ref>
    // ...
    updater.update_value(&new_value_with_comment, |p| {
        p / "upstreams" / upstream_index / git_key.as_str() / "ref"
    });
}

/// Process git upstreams and replace refs with commit hashes if they differ.
pub fn update_git_upstream_refs(recipe: &Recipe, recipe_path: &Path) -> Result<(), GitError> {
    let mut yaml_updater = yaml::Updater::new();
    let mut refs_updated = false;

    for (index, upstream) in recipe.parsed.upstreams.iter().enumerate() {
        if let stone_recipe::Upstream::Git { uri, ref_id, .. } = upstream {
            if !is_valid_commit_hash(ref_id) {
                let commit_hash = resolve_git_ref(uri, ref_id)?;
                update_git_upstream_ref_in_yaml(&mut yaml_updater, index, uri.as_str(), &commit_hash, ref_id);
                println!(
                    "{} | Updated ref '{ref_id}' to commit {} for {uri}",
                    "Warning".yellow(),
                    &commit_hash[..8],
                );
                refs_updated = true;
            }
        }
    }

    if refs_updated {
        let updated_yaml = yaml_updater.apply(&recipe.source);
        fs::write(recipe_path, updated_yaml)?;
        println!(
            "{} | Git references resolved to commit hashes and saved to stone.yaml. This ensures reproducible builds since tags and branches can move over time.",
            "Warning".yellow()
        );
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git ls-remote failed for {0}")]
    LsRemoteFailed(Url),
    #[error("could not resolve ref '{ref_id}' for {uri}. note: partial hashes are not supported")]
    UnresolvedRef { ref_id: String, uri: Url },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Utf8(#[from] string::FromUtf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_git_upstream_yaml_short_format() {
        let yaml_content = r#"upstreams:
  - git|https://github.com/user/repo: v1.0.0"#;

        let mut updater = yaml::Updater::new();
        update_git_upstream_ref_in_yaml(
            &mut updater,
            0,
            "https://github.com/user/repo",
            "abc123def456",
            "v1.0.0",
        );

        let result = updater.apply(yaml_content);
        assert!(result.contains("abc123def456 # v1.0.0"));
    }

    #[test]
    fn test_update_git_upstream_yaml_long_format() {
        let yaml_content = r#"upstreams:
  - git|https://github.com/user/repo:
      ref: v1.0.0
      staging: true"#;

        let mut updater = yaml::Updater::new();
        update_git_upstream_ref_in_yaml(
            &mut updater,
            0,
            "https://github.com/user/repo",
            "abc123def456",
            "v1.0.0",
        );

        let result = updater.apply(yaml_content);
        assert!(result.contains("abc123def456 # v1.0.0"));
        assert!(result.contains("staging: true"));
    }

    #[test]
    fn test_update_git_upstream_yaml_no_matching_ref() {
        let yaml_content = r#"upstreams:
  - git|https://github.com/other/repo: v2.0.0"#;

        let mut updater = yaml::Updater::new();
        update_git_upstream_ref_in_yaml(
            &mut updater,
            0,
            "https://github.com/user/repo",
            "abc123def456",
            "v1.0.0",
        );

        let result = updater.apply(yaml_content);
        assert!(result.contains("v2.0.0"));
        assert!(!result.contains("abc123def456"));
    }
}
