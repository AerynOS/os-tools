// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::{
    ffi::OsStr,
    fs::Metadata,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
};

use astr::AStr;
use fs_err as fs;
use glob::Pattern;
use nix::libc::{S_IFDIR, S_IRGRP, S_IROTH, S_IRWXU, S_IXGRP, S_IXOTH};
use stone::{StoneDigestWriter, StoneDigestWriterHasher, StonePayloadLayoutFile, StonePayloadLayoutRecord};
use thiserror::Error;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Rule {
    pub pattern: String,
    pub package: String,
}

impl Rule {
    pub fn matches(&self, path: &str) -> bool {
        if self.pattern == path {
            return true;
        }

        // Escape the directory in case it contains characters that have special
        // meaning in glob patterns (e.g., `[` or `]`).
        let escaped_path = Pattern::escape(path);
        Pattern::new(&self.pattern)
                .map(|pattern| pattern.matches(&escaped_path))
                .unwrap_or_default()
            // If the supplied pattern is for a directory we want to match anything that's inside said directory,
            // Do this by creating a recursive glob pattern by appending `**` if the pattern already ends in a `/` or `/**` if not
            || Pattern::new(format!("{}/**", &self.pattern.strip_suffix("/").unwrap_or(&self.pattern)).as_str())
                .map(|pattern| pattern.matches(&escaped_path))
                .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct Collector {
    /// Rules stored in order of
    /// ascending priority
    rules: Vec<Rule>,
    root: PathBuf,
}

impl Collector {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            rules: vec![],
            root: root.into(),
        }
    }

    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    fn matching_package(&self, path: &str) -> Option<&str> {
        // Rev = check highest priority rules first
        self.rules
            .iter()
            .rev()
            .find_map(|rule| rule.matches(path).then_some(rule.package.as_str()))
    }

    /// Produce a [`PathInfo`] from the provided [`Path`]
    pub fn path(&self, path: &Path, hasher: &mut StoneDigestWriterHasher) -> Result<PathInfo, Error> {
        let metadata = fs::metadata(path)?;
        self.path_with_metadata(path.to_path_buf(), &metadata, hasher)
    }

    fn path_with_metadata(
        &self,
        path: PathBuf,
        metadata: &Metadata,
        hasher: &mut StoneDigestWriterHasher,
    ) -> Result<PathInfo, Error> {
        let target_path = Path::new("/").join(path.strip_prefix(&self.root).expect("path is ancestor of root"));

        let package = self
            .matching_package(target_path.to_str().unwrap_or_default())
            .ok_or(Error::NoMatchingRule)?;

        PathInfo::new(path, target_path, metadata, hasher, package.to_owned())
    }

    /// Enumerates all paths from the filesystem starting at root or subdir of root, if provided
    pub fn enumerate_paths(
        &self,
        subdir: Option<(PathBuf, Metadata)>,
        hasher: &mut StoneDigestWriterHasher,
    ) -> Result<Vec<PathInfo>, Error> {
        let mut paths = vec![];

        let dir = subdir.as_ref().map(|t| t.0.as_path()).unwrap_or(&self.root);
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;

        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let metadata = entry.metadata()?;

            let host_path = entry.path();

            if metadata.is_dir() {
                paths.extend(self.enumerate_paths(Some((host_path, metadata)), hasher)?);
            } else {
                paths.push(self.path_with_metadata(host_path, &metadata, hasher)?);
            }
        }

        // Include empty or special dir
        //
        // Regular 755 dir w/ entries can be
        // recreated when adding children
        if let Some((dir, meta)) = subdir {
            const REGULAR_DIR_MODE: u32 = S_IFDIR | S_IROTH | S_IXOTH | S_IRGRP | S_IXGRP | S_IRWXU;

            let is_special = meta.mode() != REGULAR_DIR_MODE;

            if meta.is_dir() && (paths.is_empty() || is_special) {
                paths.push(self.path_with_metadata(dir, &meta, hasher)?);
            }
        }

        Ok(paths)
    }
}

#[derive(Debug)]
pub struct PathInfo {
    pub path: PathBuf,
    pub target_path: PathBuf,
    pub layout: StonePayloadLayoutRecord,
    pub size: u64,
    pub package: String,
}

impl PathInfo {
    pub fn new(
        path: PathBuf,
        target_path: PathBuf,
        metadata: &Metadata,
        hasher: &mut StoneDigestWriterHasher,
        package: String,
    ) -> Result<Self, Error> {
        let layout = layout_from_metadata(&path, &target_path, metadata, hasher)?;

        Ok(Self {
            path,
            target_path,
            layout,
            size: metadata.size(),
            package,
        })
    }

    pub fn restat(&mut self, hasher: &mut StoneDigestWriterHasher) -> Result<(), Error> {
        let metadata = fs::metadata(&self.path)?;
        self.layout = layout_from_metadata(&self.path, &self.target_path, &metadata, hasher)?;
        self.size = metadata.size();
        Ok(())
    }

    pub fn is_file(&self) -> bool {
        matches!(self.layout.file, StonePayloadLayoutFile::Regular(_, _))
    }

    pub fn file_hash(&self) -> Option<u128> {
        if let StonePayloadLayoutFile::Regular(hash, _) = &self.layout.file {
            Some(*hash)
        } else {
            None
        }
    }

    pub fn file_name(&self) -> &str {
        self.target_path
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or_default()
    }

    pub fn has_component(&self, component: &str) -> bool {
        self.target_path
            .components()
            .any(|c| c.as_os_str() == OsStr::new(component))
    }
}

fn layout_from_metadata(
    path: &Path,
    target_path: &Path,
    metadata: &Metadata,
    hasher: &mut StoneDigestWriterHasher,
) -> Result<StonePayloadLayoutRecord, Error> {
    // Strip /usr
    let target: AStr = target_path
        .strip_prefix("/usr")
        .unwrap_or(target_path)
        .to_string_lossy()
        .into();

    let file_type = metadata.file_type();

    Ok(StonePayloadLayoutRecord {
        uid: metadata.uid(),
        gid: metadata.gid(),
        mode: metadata.mode(),
        tag: 0,
        file: if file_type.is_symlink() {
            let source = fs::read_link(path)?;

            StonePayloadLayoutFile::Symlink(source.to_string_lossy().into(), target)
        } else if file_type.is_dir() {
            StonePayloadLayoutFile::Directory(target)
        } else if file_type.is_char_device() {
            StonePayloadLayoutFile::CharacterDevice(target)
        } else if file_type.is_block_device() {
            StonePayloadLayoutFile::BlockDevice(target)
        } else if file_type.is_fifo() {
            StonePayloadLayoutFile::Fifo(target)
        } else if file_type.is_socket() {
            StonePayloadLayoutFile::Socket(target)
        } else {
            hasher.reset();

            let mut digest_writer = StoneDigestWriter::new(io::sink(), hasher);
            let mut file = fs::File::open(path)?;

            // Copy bytes to null sink so we don't
            // explode memory
            io::copy(&mut file, &mut digest_writer)?;

            let hash = hasher.digest128();

            StonePayloadLayoutFile::Regular(hash, target)
        },
    })
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("no matching path rule")]
    NoMatchingRule,
    #[error("io")]
    Io(#[from] io::Error),
}
