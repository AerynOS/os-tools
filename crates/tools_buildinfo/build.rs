// build.rs
use std::os::unix::ffi::OsStringExt;

use chrono::{DateTime, Utc};

/// Returns value of given environment variable or error if missing.
///
/// This also outputs necessary ‘cargo:rerun-if-env-changed’ tag to make sure
/// build script is rerun if the environment variable changes.
fn env(key: &str) -> Result<std::ffi::OsString, Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed={key}");
    std::env::var_os(key).ok_or_else(|| Box::from(format!("Missing `{key}` environmental variable")))
}

/// Calls program with given arguments and returns its standard output.  If
/// calling the program fails or it exits with non-zero exit status returns an
/// error.
fn command(prog: &str, args: &[&str], cwd: Option<std::path::PathBuf>) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=PATH");
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args);
    cmd.stderr(std::process::Stdio::inherit());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let out = cmd.output()?;
    if out.status.success() {
        let mut stdout = out.stdout;
        if let Some(b'\n') = stdout.last() {
            stdout.pop();
            if let Some(b'\r') = stdout.last() {
                stdout.pop();
            }
        }
        Ok(stdout)
    } else if let Some(code) = out.status.code() {
        Err(Box::from(format!("{prog}: terminated with {code}")))
    } else {
        Err(Box::from(format!("{prog}: killed by signal")))
    }
}

/// Checks to see if we're building from a git source and if so attempts to gather information about the git status
fn get_git_info() -> Result<(), Box<dyn std::error::Error>> {
    // These are cfgs that can be set by this script. We need to declare them always to ensure that clippy is happy
    println!("cargo:rustc-check-cfg=cfg(BUILDINFO_IS_DIRTY)");
    println!("cargo:rustc-check-cfg=cfg(BUILDINFO_IS_GIT_BUILD)");

    let pkg_dir = std::path::PathBuf::from(env("CARGO_MANIFEST_DIR")?);
    let git_dir = command("git", &["rev-parse", "--git-dir"], Some(pkg_dir.clone()));
    let git_dir = match git_dir {
        Ok(git_dir) => {
            println!("cargo:rustc-cfg=BUILDINFO_IS_GIT_BUILD");

            std::path::PathBuf::from(std::ffi::OsString::from_vec(git_dir))
        }
        Err(msg) => {
            // We're not in a git repo, most likely we're building from a source archive
            println!("cargo:warning=unable to determine git version (not in git repository?)");
            println!("cargo:warning={msg}");

            // It's unlikely, but possible that someone could run git init. Might as well catch that.
            println!("cargo::rerun-if-changed={}/.git", pkg_dir.display());
            return Ok(());
        }
    };

    // Make Cargo rerun us if currently checked out commit or the state of the
    // working tree changes.  We try to accomplish that by looking at a few
    // crucial git state files.  This probably may result in some false
    // negatives but it’s best we’ve got.
    for subpath in ["HEAD", "logs/HEAD", "index"] {
        let path = git_dir.join(subpath).canonicalize()?;
        println!("cargo:rerun-if-changed={}", path.display());
    }

    // Get the full git hash
    let args = &["rev-parse", "--output-object-format=sha1", "HEAD"];
    let out = command("git", args, None)?;
    match String::from_utf8_lossy(&out) {
        std::borrow::Cow::Borrowed(full_hash) => {
            println!("cargo:rustc-env=BUILDINFO_GIT_FULL_HASH={}", full_hash.trim());
        }
        std::borrow::Cow::Owned(full_hash) => return Err(Box::from(format!("git: Invalid output: {full_hash}"))),
    }

    // Get the short git hash
    let args = &["rev-parse", "--output-object-format=sha1", "--short", "HEAD"];
    let out = command("git", args, None)?;
    match String::from_utf8_lossy(&out) {
        std::borrow::Cow::Borrowed(short_hash) => {
            println!("cargo:rustc-env=BUILDINFO_GIT_SHORT_HASH={}", short_hash.trim());
        }
        std::borrow::Cow::Owned(short_hash) => return Err(Box::from(format!("git: Invalid output: {short_hash}"))),
    }

    // Get whether this is built from a dirty tree
    let args = &["status", "--porcelain"];
    let out = command("git", args, None)?;
    match String::from_utf8_lossy(&out) {
        std::borrow::Cow::Borrowed(output) => match output.trim().len() {
            0 => {}
            _ => println!("cargo:rustc-cfg=BUILDINFO_IS_DIRTY"),
        },
        std::borrow::Cow::Owned(output) => return Err(Box::from(format!("git: Invalid output: {output}"))),
    }

    // Get the commit summary
    let args = &["show", "--format=\"%s\"", "-s"];
    let out = command("git", args, None)?;
    match String::from_utf8_lossy(&out) {
        std::borrow::Cow::Borrowed(summary) => {
            println!("cargo:rustc-env=BUILDINFO_GIT_SUMMARY={}", summary.trim());
        }
        std::borrow::Cow::Owned(summary) => return Err(Box::from(format!("git: Invalid output: {summary}"))),
    }

    Ok(())
}

fn get_build_time() -> Result<(), Box<dyn std::error::Error>> {
    // Propagate SOURCE_DATE_EPOCH if set
    if let Ok(epoch_env) = env("SOURCE_DATE_EPOCH")
        && let Ok(seconds) = epoch_env.to_string_lossy().parse::<i64>()
        && let Some(time) = DateTime::from_timestamp(seconds, 0)
    {
        println!("cargo:rustc-env=BUILDINFO_BUILD_TIME={}", time.timestamp());
        return Ok(());
    }

    println!("cargo:rustc-env=BUILDINFO_BUILD_TIME={}", Utc::now().timestamp());
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // This should include all top-level directories that contain source code or otherwise modify the build in meaningful ways
    let top_level = std::path::PathBuf::from("../..").canonicalize()?;
    println!("cargo::rerun-if-changed={}/boulder", top_level.display());
    println!("cargo::rerun-if-changed={}/crates", top_level.display());
    println!("cargo::rerun-if-changed={}/moss", top_level.display());
    println!("cargo::rerun-if-changed={}/test", top_level.display());
    println!("cargo::rerun-if-changed={}/Cargo.toml", top_level.display());

    let version = env("CARGO_PKG_VERSION")?;
    println!("cargo:rustc-env=BUILDINFO_VERSION={}", version.to_string_lossy());

    get_build_time()?;

    get_git_info()?;

    Ok(())
}
