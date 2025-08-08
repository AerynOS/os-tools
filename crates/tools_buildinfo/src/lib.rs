// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use chrono::DateTime;

mod values;

/// Returns the version of the project, as defined in the top-level Cargo.toml
///
/// This will look like "0.1.0"
pub const fn get_version() -> &'static str {
    values::VERSION
}

/// Returns the build time of the project, printed in UTC time format
///
/// If SOURCE_DATE_EPOCH is set during the build then that will be the timestamp returned
///
/// This will look like "2025-07-09T19:20:40+00:00"
pub fn get_build_time() -> String {
    if let Ok(time) = values::BUILD_TIME.parse::<i64>() {
        if let Some(build_time) = DateTime::from_timestamp(time, 0) {
            return build_time.to_rfc3339();
        }
    }
    "unknown".to_owned()
}

/// Returns `true` if the project was built from a git source, `false` otherwise
pub const fn get_if_git_build() -> bool {
    cfg!(BUILDINFO_IS_GIT_BUILD)
}

/// Returns `-dirty` if the project was built from a dirty git source, `` otherwise
pub const fn get_git_dirty() -> &'static str {
    if cfg!(BUILDINFO_IS_DIRTY) { "-dirty" } else { "" }
}

/// Returns the git hash that the project was built from if built from a git source
///
/// This currently returns the SHA1 hash, though eventually it will return the SHA256 one
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub const fn get_git_full_hash() -> &'static str {
    values::GIT_FULL_HASH
}

/// Returns the git hash that the project was built from if built from a git source
///
/// This currently returns the SHA1 hash, though eventually it will return the SHA256 one
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(not(BUILDINFO_IS_GIT_BUILD))]
pub const fn get_git_full_hash() -> &'static str {
    "unknown"
}

/// Returns the shortened form of the git hash that this project was built from if built from git source
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub const fn get_git_short_hash() -> &'static str {
    values::GIT_SHORT_HASH
}

/// Returns the shortened form of the git hash that this project was built from if built from git source
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(not(BUILDINFO_IS_GIT_BUILD))]
pub const fn get_git_short_hash() -> &'static str {
    "unknown"
}

/// Returns the summary of the git commit that the project was built from
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub const fn get_git_summary() -> &'static str {
    values::GIT_SUMMARY
}

/// Returns the summary of the git commit that the project was built from
///
/// If built from a non-git source (like a release archive) this will return "unknown"
#[cfg(not(BUILDINFO_IS_GIT_BUILD))]
pub const fn get_git_summary() -> &'static str {
    "unknown"
}

/// For git builds this returns a string like `v0.1.0 (git 4ecad5d7e70c2cdc81350dc6b46fb55b1ccb18f5-dirty)`
///
/// For builds from a non-git source just the version will be returned: `v0.1.0`
pub fn get_simple_version() -> String {
    let git = if cfg!(BUILDINFO_IS_GIT_BUILD) {
        format!(" (Git ref {}{})", get_git_full_hash(), get_git_dirty())
    } else {
        "".to_owned()
    };
    format!("v{}{git}", values::VERSION)
}

/// For git builds this returns a string like `version v0.1.0 (git 4ecad5d7e70c2cdc81350dc6b46fb55b1ccb18f5-dirty) (Built at 2025-07-09T19:20:40+00:00)`
///
/// For builds from a non-git source just the version will be returned: `version v0.1.0 (Built at 2025-07-09T19:20:40+00:00)`
pub fn get_full_version() -> String {
    let git = if cfg!(BUILDINFO_IS_GIT_BUILD) {
        format!(" (Git ref {}{})", get_git_full_hash(), get_git_dirty())
    } else {
        "".to_owned()
    };
    format!("version v{}{git} (Built at {})", values::VERSION, get_build_time())
}
