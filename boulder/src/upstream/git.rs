// SPDX-FileCopyrightText: Copyright © 2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs, io,
    path::{Path, PathBuf},
    process, string,
};

use moss::{runtime, util};
use thiserror::Error;
use tui::{ProgressBar, ProgressStyle, Styled};
use url::Url;

use crate::{Paths, upstream::Stored};

#[derive(Clone, Debug)]
pub struct Git {
    pub uri: Url,
    pub ref_id: String,
    pub staging: bool,
    pub original_index: usize,
}

impl Git {
    pub async fn fetch_new(url: &Url, dest_dir: &Path) -> Result<Self, Error> {
        Self::fetch_new_progress(&url, dest_dir, &ProgressBar::hidden()).await
    }

    pub async fn fetch_new_progress(url: &Url, dest_dir: &Path, pb: &ProgressBar) -> Result<Self, Error> {
        todo!()
    }

    pub fn name(&self) -> &str {
        util::uri_file_name(&self.uri)
    }

    fn final_path(&self, paths: &Paths) -> PathBuf {
        paths
            .upstreams()
            .host
            .join("git")
            .join(util::uri_relative_path(&self.uri))
    }

    fn staging_path(&self, paths: &Paths) -> PathBuf {
        paths
            .upstreams()
            .host
            .join("staging")
            .join("git")
            .join(util::uri_relative_path(&self.uri))
    }

    pub async fn store(&self, paths: &Paths, pb: &ProgressBar) -> Result<StoredGit, Error> {
        use tokio::fs;

        pb.set_style(
            ProgressStyle::with_template(" {spinner} {wide_msg} ")
                .unwrap()
                .tick_chars("--=≡■≡=--"),
        );

        let clone_path = if self.staging {
            self.staging_path(paths)
        } else {
            self.final_path(paths)
        };
        let clone_path_string = clone_path.display().to_string();

        let final_path = self.final_path(paths);
        let final_path_string = final_path.display().to_string();

        if let Some(parent) = clone_path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }
        if let Some(parent) = final_path.parent().map(Path::to_path_buf) {
            runtime::unblock(move || util::ensure_dir_exists(&parent)).await?;
        }

        if self.ref_exists(&final_path).await? {
            self.reset_to_ref(&final_path).await?;
            let resolved_hash = runtime::unblock({
                let final_path = final_path.clone();
                let ref_id = self.ref_id.clone();
                let uri = self.uri.clone();
                move || resolve_git_ref(&final_path, &ref_id, &uri)
            })
            .await?;
            return Ok(StoredGit {
                name: self.name().to_owned(),
                path: final_path,
                was_cached: true,
                uri: self.uri.clone(),
                original_ref: self.ref_id.clone(),
                resolved_hash,
                original_index: self.original_index,
            });
        }

        let _ = fs::remove_dir_all(&clone_path).await;
        if self.staging {
            let _ = fs::remove_dir_all(&final_path).await;
        }

        let mut args = vec!["clone"];
        if self.staging {
            args.push("--mirror");
        }
        args.extend(["--", self.uri.as_str(), &clone_path_string]);

        self.run(&args, None).await?;

        if self.staging {
            self.run(&["clone", "--", &clone_path_string, &final_path_string], None)
                .await?;
        }

        self.reset_to_ref(&final_path).await?;

        let resolved_hash = runtime::unblock({
            let final_path = final_path.clone();
            let ref_id = self.ref_id.clone();
            let uri = self.uri.clone();
            move || resolve_git_ref(&final_path, &ref_id, &uri)
        })
        .await?;

        Ok(StoredGit {
            name: self.name().to_owned(),
            path: final_path,
            was_cached: false,
            uri: self.uri.clone(),
            original_ref: self.ref_id.clone(),
            resolved_hash,
            original_index: self.original_index,
        })
    }

    async fn ref_exists(&self, path: &Path) -> Result<bool, Error> {
        if !path.exists() {
            return Ok(false);
        }

        self.run(&["fetch"], Some(path)).await?;

        let result = self.run(&["cat-file", "-e", &self.ref_id], Some(path)).await;

        Ok(result.is_ok())
    }

    async fn reset_to_ref(&self, path: &Path) -> Result<(), Error> {
        self.run(&["reset", "--hard", &self.ref_id], Some(path)).await?;

        self.run(
            &[
                "submodule",
                "update",
                "--init",
                "--recursive",
                "--depth",
                "1",
                "--jobs",
                "4",
            ],
            Some(path),
        )
        .await?;

        Ok(())
    }

    async fn run(&self, args: &[&str], cwd: Option<&Path>) -> Result<(), Error> {
        use tokio::process;

        let mut command = process::Command::new("git");

        if let Some(dir) = cwd {
            command.current_dir(dir);
        }

        let output = command.args(args).output().await?;

        if !output.status.success() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return Err(Error::GitFailed(self.uri.clone()));
        }

        Ok(())
    }

    pub fn remove(&self, paths: &Paths) -> Result<(), Error> {
        for path in [self.staging_path(paths), self.final_path(paths)] {
            fs::remove_dir_all(&path)?;

            if let Some(parent) = path.parent() {
                util::remove_empty_dirs(parent, &paths.upstreams().host)?;
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct StoredGit {
    pub name: String,
    pub path: PathBuf,
    pub was_cached: bool,
    pub uri: Url,
    pub original_ref: String,
    pub resolved_hash: String,
    pub original_index: usize,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to clone {0}")]
    GitFailed(Url),
    #[error("git")]
    Git(#[from] GitError),
    #[error("io")]
    Io(#[from] io::Error),
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

/// Process git upstreams after cloning and return updated YAML if refs differ from resolved hashes.
pub(crate) fn update_git_upstream_refs(
    recipe_source: &str,
    stored_upstreams: &[Stored],
) -> Result<Option<String>, GitError> {
    let mut yaml_updater = yaml::Updater::new();
    let mut refs_updated = false;

    for stored in stored_upstreams.iter() {
        if let Stored::Git(git) = stored
            && git.resolved_hash != git.original_ref
        {
            update_git_upstream_ref_in_yaml(
                &mut yaml_updater,
                git.original_index,
                git.uri.as_str(),
                &git.resolved_hash,
                &git.original_ref,
            );
            println!(
                "{} | Updated ref '{}' to commit {} for {}",
                "Warning".yellow(),
                &git.resolved_hash[..8],
                &git.original_ref,
                &git.uri
            );
            refs_updated = true;
        }
    }

    if refs_updated {
        Ok(Some(yaml_updater.apply(recipe_source)))
    } else {
        Ok(None)
    }
}

fn is_valid_commit_hash(s: &str) -> bool {
    // git commit hashes can be SHA-1 or SHA-256 hashes
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
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

#[cfg(test)]
mod tests {
    use crate::upstream::StoredGit;
    use crate::upstream::plain::StoredPlain;

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

        let stored = vec![
            Stored::Git(StoredGit {
                name: "repo1.git".to_owned(),
                path: "/tmp/repo1".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo1.git").unwrap(),
                original_ref: "main".to_owned(),
                resolved_hash: "1111222233334444555566667777888899990000".to_owned(),
                original_index: 0,
            }),
            Stored::Git(StoredGit {
                name: "repo2.git".to_owned(),
                path: "/tmp/repo2".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo2.git").unwrap(),
                original_ref: "main".to_owned(),
                resolved_hash: "aaaa1111bbbb2222cccc3333dddd4444eeee5555".to_owned(),
                original_index: 1,
            }),
            Stored::Git(StoredGit {
                name: "repo3.git".to_owned(),
                path: "/tmp/repo3".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo3.git").unwrap(),
                original_ref: "abcd1234567890abcdef1234567890abcdef1234".to_owned(),
                resolved_hash: "abcd1234567890abcdef1234567890abcdef1234".to_owned(),
                original_index: 2,
            }),
            Stored::Git(StoredGit {
                name: "repo4.git".to_owned(),
                path: "/tmp/repo4".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo4.git").unwrap(),
                original_ref: "abc123d".to_owned(),
                resolved_hash: "abc123d567890abcdef1234567890abcdef12345".to_owned(),
                original_index: 3,
            }),
            Stored::Git(StoredGit {
                name: "file.tar.gz".to_owned(),
                path: "/tmp/file.tar.gz".into(),
                was_cached: false,
                // We don't care about the values below.
                uri: "http://example.com".try_into().unwrap(),
                original_ref: String::new(),
                resolved_hash: String::new(),
                original_index: 0,
            }),
        ];

        let result = update_git_upstream_refs(recipe_source, &stored).unwrap();

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

        let stored = vec![
            Stored::Git(StoredGit {
                name: "repo3.git".to_owned(),
                path: "/tmp/repo3".into(),
                was_cached: false,
                uri: Url::parse("https://github.com/example/repo3.git").unwrap(),
                original_ref: "abcd1234567890abcdef1234567890abcdef1234".to_owned(),
                resolved_hash: "abcd1234567890abcdef1234567890abcdef1234".to_owned(),
                original_index: 0,
            }),
            Stored::Plain(StoredPlain {
                name: "file.tar.gz".to_owned(),
                path: "/tmp/file.tar.gz".into(),
                was_cached: false,
            }),
        ];

        let result = update_git_upstream_refs(recipe_source, &stored).unwrap();

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
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["config", "user.email", "test@test.com"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["config", "user.name", "Test"])
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
        let commit_hash = String::from_utf8(output.stdout).unwrap().trim().to_owned();

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
