// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::Path, process, string};

use crate::build::upstream::Installed;
use thiserror::Error;
use tui::Styled;
use url::Url;
use yaml;

fn is_valid_commit_hash(s: &str) -> bool {
    // git commit hashes can be SHA-1 or SHA-256 hashes
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolves a git reference to its commit hash using `git rev-parse` on a cloned repo.
pub(crate) fn resolve_git_ref(clone_dir: &Path, ref_id: &str, uri: &Url) -> Result<String, GitError> {
    let output = process::Command::new("git")
        .current_dir(clone_dir)
        .args(["rev-parse", ref_id])
        .output()?;

    if !output.status.success() {
        return Err(GitError::UnresolvedRef {
            ref_id: ref_id.to_owned(),
            uri: uri.clone(),
        });
    }

    let stdout = String::from_utf8(output.stdout)?;
    let parsed_hash = stdout.trim();

    if !is_valid_commit_hash(parsed_hash) {
        return Err(GitError::UnresolvedRef {
            ref_id: ref_id.to_owned(),
            uri: uri.clone(),
        });
    }

    Ok(parsed_hash.to_owned())
}

/// Replaces the non-hash refs for git upstreams with the hash for the given ref
/// and includes a comment showing the original ref.
fn update_git_upstream_ref_in_yaml(
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

/// Process git upstreams after cloning and return updated YAML if refs differ from resolved hashes.
pub(crate) fn update_git_upstream_refs(
    recipe_source: &str,
    installed_upstreams: &[Installed],
) -> Result<Option<String>, GitError> {
    let mut yaml_updater = yaml::Updater::new();
    let mut refs_updated = false;

    for installed in installed_upstreams.iter() {
        if let Installed::Git {
            uri,
            original_ref,
            resolved_hash,
            original_index,
            ..
        } = installed
        {
            if resolved_hash != original_ref {
                update_git_upstream_ref_in_yaml(
                    &mut yaml_updater,
                    *original_index,
                    uri.as_str(),
                    resolved_hash,
                    original_ref,
                );
                println!(
                    "{} | Updated ref '{original_ref}' to commit {} for {uri}",
                    "Warning".yellow(),
                    &resolved_hash[..8],
                );
                refs_updated = true;
            }
        }
    }

    if refs_updated {
        Ok(Some(yaml_updater.apply(recipe_source)))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("ref '{ref_id}' did not resolve to a valid commit hash for {uri}")]
    UnresolvedRef { ref_id: String, uri: Url },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Utf8(#[from] string::FromUtf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn test_update_git_upstream_refs() {
        let recipe_source = r#"
upstreams:
  - git|https://github.com/example/repo1.git: main
  - git|https://github.com/example/repo2.git:
      ref: main
  - git|https://github.com/example/repo3.git: abcd1234567890abcdef1234567890abcdef1234
  - git|https://github.com/example/repo4.git: abc123d
  - https://example.com/file.tar.gz: some-hash
"#;

        let installed = vec![
            Installed::Git {
                name: "repo1.git".to_string(),
                path: "/tmp/repo1".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo1.git").unwrap(),
                original_ref: "main".to_string(),
                resolved_hash: "1111222233334444555566667777888899990000".to_string(),
                original_index: 0,
            },
            Installed::Git {
                name: "repo2.git".to_string(),
                path: "/tmp/repo2".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo2.git").unwrap(),
                original_ref: "main".to_string(),
                resolved_hash: "aaaa1111bbbb2222cccc3333dddd4444eeee5555".to_string(),
                original_index: 1,
            },
            Installed::Git {
                name: "repo3.git".to_string(),
                path: "/tmp/repo3".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo3.git").unwrap(),
                original_ref: "abcd1234567890abcdef1234567890abcdef1234".to_string(),
                resolved_hash: "abcd1234567890abcdef1234567890abcdef1234".to_string(),
                original_index: 2,
            },
            Installed::Git {
                name: "repo4.git".to_string(),
                path: "/tmp/repo4".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo4.git").unwrap(),
                original_ref: "abc123d".to_string(),
                resolved_hash: "abc123d567890abcdef1234567890abcdef12345".to_string(),
                original_index: 3,
            },
            Installed::Plain {
                name: "file.tar.gz".to_string(),
                path: "/tmp/file.tar.gz".into(),
                was_cached: false,
            },
        ];

        let result = update_git_upstream_refs(recipe_source, &installed).unwrap();

        assert!(result.is_some());
        let updated_yaml = result.unwrap();

        // Should update short form ref to hash with comment
        assert!(updated_yaml.contains("1111222233334444555566667777888899990000 # main"));

        // Should update long form ref to hash with comment
        assert!(updated_yaml.contains("aaaa1111bbbb2222cccc3333dddd4444eeee5555 # main"));

        // Should not change hash that's already a hash
        assert!(updated_yaml.contains("abcd1234567890abcdef1234567890abcdef1234"));
        assert!(
            !updated_yaml
                .contains("abcd1234567890abcdef1234567890abcdef1234 # abcd1234567890abcdef1234567890abcdef1234")
        );

        // Should update short hash to long hash
        assert!(updated_yaml.contains("abc123d567890abcdef1234567890abcdef12345 # abc123d"));

        // Should preserve non-git upstreams unchanged
        assert!(updated_yaml.contains("https://example.com/file.tar.gz: some-hash"));
    }

    #[test]
    fn test_update_git_upstream_refs_no_updates() {
        let recipe_source = r#"
upstreams:
  - git|https://github.com/example/repo3.git: abcd1234567890abcdef1234567890abcdef1234
  - https://example.com/file.tar.gz: some-hash
"#;

        let installed = vec![
            Installed::Git {
                name: "repo3.git".to_string(),
                path: "/tmp/repo3".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo3.git").unwrap(),
                original_ref: "abcd1234567890abcdef1234567890abcdef1234".to_string(),
                resolved_hash: "abcd1234567890abcdef1234567890abcdef1234".to_string(),
                original_index: 0,
            },
            Installed::Plain {
                name: "file.tar.gz".to_string(),
                path: "/tmp/file.tar.gz".into(),
                was_cached: false,
            },
        ];

        let result = update_git_upstream_refs(recipe_source, &installed).unwrap();

        assert!(result.is_none());
    }

    // Create a minimal test repo
    fn setup_test_repo() -> (TempDir, String) {
        let temp_dir = TempDir::new().unwrap();

        // Initialize the repo
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["init"])
            .output()
            .unwrap();

        // Create the first commit
        fs::write(temp_dir.path().join("file"), "content").unwrap();
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["add", "."])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["commit", "-m", "test"])
            .output()
            .unwrap();

        // Create a tag for testing
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["tag", "v1.0"])
            .output()
            .unwrap();

        // Get the commit hash for testing
        let output = Command::new("git")
            .current_dir(temp_dir.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let commit_hash = String::from_utf8(output.stdout).unwrap().trim().to_string();

        (temp_dir, commit_hash)
    }

    #[test]
    fn test_resolve_invalid_repo_path() {
        let uri = Url::parse("https://example.com/test.git").unwrap();

        let err = resolve_git_ref(Path::new("/nonexistent"), "v1.0", &uri).unwrap_err();

        assert!(matches!(err, GitError::Io(_)));
    }

    #[test]
    fn test_resolve_tag() {
        let (temp_dir, expected_hash) = setup_test_repo();
        let uri = Url::parse("https://example.com/test.git").unwrap();

        let result = resolve_git_ref(temp_dir.path(), "v1.0", &uri).unwrap();

        assert_eq!(result, expected_hash);
    }

    #[test]
    fn test_resolve_short_hash() {
        let (temp_dir, full_hash) = setup_test_repo();
        let uri = Url::parse("https://example.com/test.git").unwrap();
        let short_hash = &full_hash[..8];

        let result = resolve_git_ref(temp_dir.path(), short_hash, &uri).unwrap();

        assert_eq!(result, full_hash);
    }

    #[test]
    fn test_resolve_full_hash() {
        let (temp_dir, full_hash) = setup_test_repo();
        let uri = Url::parse("https://example.com/test.git").unwrap();

        let result = resolve_git_ref(temp_dir.path(), &full_hash, &uri).unwrap();

        assert_eq!(result, full_hash);
    }

    #[test]
    fn test_resolve_invalid_ref() {
        let (temp_dir, _) = setup_test_repo();
        let uri = Url::parse("https://example.com/test.git").unwrap();

        let err = resolve_git_ref(temp_dir.path(), "nonexistent", &uri).unwrap_err();

        assert!(matches!(err, GitError::UnresolvedRef { .. }));
    }
}
