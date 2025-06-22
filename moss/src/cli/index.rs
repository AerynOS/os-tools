// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use std::{
    collections::{BTreeMap, btree_map},
    io,
    path::{Path, PathBuf, StripPrefixError},
    time::Duration,
};

use camino::Utf8Path;
use clap::{ArgMatches, Command, arg, value_parser};
use fs_err as fs;
use moss::{
    client,
    package::{self, Meta, MissingMetaFieldError},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

pub fn command() -> Command {
    Command::new("index")
        .visible_alias("ix")
        .about("Index a collection of packages")
        .arg(arg!(<INDEX_DIR> "directory of index files").value_parser(value_parser!(PathBuf)))
}

pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let index_dir = args.get_one::<PathBuf>("INDEX_DIR").unwrap().canonicalize()?;

    let stone_files = enumerate_stone_files(&index_dir)?;

    println!("Indexing {} files\n", stone_files.len());

    let multi_progress = MultiProgress::new();

    let total_progress = multi_progress.add(
        ProgressBar::new(stone_files.len() as u64).with_style(
            ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
                .unwrap()
                .progress_chars("■≡=- "),
        ),
    );
    total_progress.tick();

    let ctx = GetMetaCtx {
        index_dir: &index_dir,
        multi_progress: &multi_progress,
        total_progress: &total_progress,
    };
    let list = stone_files
        .par_iter()
        .map(|path| get_meta(path, ctx))
        .collect::<Result<Vec<_>, _>>()?;

    let mut map = BTreeMap::new();

    // Add each meta to the map, removing
    // dupes by keeping the latest release
    for meta in list {
        match map.entry(meta.name.clone()) {
            btree_map::Entry::Vacant(entry) => {
                entry.insert(meta);
            }
            btree_map::Entry::Occupied(mut entry) => {
                match (entry.get().source_release, meta.source_release) {
                    // Error if dupe is same version
                    (prev, curr) if prev == curr => {
                        return Err(Error::DuplicateRelease(meta.name.clone(), meta.source_release));
                    }
                    // Update if dupe is newer version
                    (prev, curr) if prev < curr => {
                        entry.insert(meta);
                    }
                    // Otherwise prev is more recent, don't replace
                    _ => {}
                }
            }
        }
    }

    write_index(&index_dir, map, &total_progress)?;

    multi_progress.clear()?;

    println!("\nIndex file written to {:?}", index_dir.join("stone.index").display());

    Ok(())
}

fn write_index(dir: &Path, map: BTreeMap<package::Name, Meta>, total_progress: &ProgressBar) -> Result<(), Error> {
    total_progress.set_message("Writing index file");
    total_progress.set_style(
        ProgressStyle::with_template("\n {spinner} {wide_msg}")
            .unwrap()
            .tick_chars("--=≡■≡=--"),
    );
    total_progress.enable_steady_tick(Duration::from_millis(150));

    let path = dir.join("stone.index");
    let mut file = fs::File::create(&path)?;

    let write_stone_index = || {
        let mut writer = stone::Writer::new(&mut file, stone::header::v1::FileType::Repository)?;

        for (_, meta) in map {
            let payload = meta.to_stone_payload();
            writer.add_payload(payload.as_slice())?;
        }

        writer.finalize()
    };

    write_stone_index().map_err(|source| Error::StoneWrite { source, path })
}

#[derive(Clone, Copy)]
struct GetMetaCtx<'a> {
    index_dir: &'a Path,
    multi_progress: &'a MultiProgress,
    total_progress: &'a ProgressBar,
}

fn get_meta(path: &Path, ctx: GetMetaCtx<'_>) -> Result<Meta, Error> {
    let relative_path: &Utf8Path = path
        .strip_prefix(ctx.index_dir)?
        .try_into()
        .map_err(|_| Error::NonUtf8Path { path: path.to_owned() })?;

    let progress = ctx
        .multi_progress
        .insert_before(ctx.total_progress, ProgressBar::new_spinner());
    progress.enable_steady_tick(Duration::from_millis(150));

    let (size, hash) = stat_file(path, relative_path, &progress)?;

    progress.set_message(format!("{} {}", "Indexing".yellow(), relative_path.as_str().bold()));
    progress.set_style(
        ProgressStyle::with_template(" {spinner} {wide_msg}")
            .unwrap()
            .tick_chars("--=≡■≡=--"),
    );

    let read_payloads = || -> Result<Vec<_>, _> {
        let mut file = fs::File::open(path)?;
        let mut reader = stone::read(&mut file)?;
        reader.payloads()?.collect()
    };
    let payloads = read_payloads().map_err(|source| Error::StoneRead {
        source,
        path: path.to_owned(),
    })?;

    let payload = payloads
        .iter()
        .find_map(|payload| payload.meta())
        .ok_or(Error::MissingMetaPayload)?;

    let mut meta = Meta::from_stone_payload(&payload.body)?;
    meta.hash = Some(hash);
    meta.download_size = Some(size);
    meta.uri = Some(relative_path.as_str().to_owned());

    progress.finish();
    ctx.multi_progress.remove(&progress);
    ctx.multi_progress
        .suspend(|| println!("{} {}", "Indexed".green(), relative_path.as_str().bold()));
    ctx.total_progress.inc(1);

    Ok(meta)
}

fn stat_file(path: &Path, relative_path: &Utf8Path, progress: &ProgressBar) -> Result<(u64, String), Error> {
    let file = fs::File::open(path)?;
    let size = file.metadata()?.len();

    progress.set_length(size);
    progress.set_message(format!("{} {}", "Hashing".blue(), relative_path.as_str().bold()));
    progress.set_style(
        ProgressStyle::with_template(" {spinner} |{percent:>3}%| {wide_msg} {binary_bytes_per_sec:>.dim} ")
            .unwrap()
            .tick_chars("--=≡■≡=--"),
    );

    let mut hasher = Sha256::new();
    io::copy(&mut &file, &mut progress.wrap_write(&mut hasher))?;

    let hash = hex::encode(hasher.finalize());

    Ok((size, hash))
}

fn enumerate_stone_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let read_dir = fs::read_dir(dir)?;
    let mut paths = vec![];

    for entry in read_dir.flatten() {
        let path = entry.path();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            paths.extend(enumerate_stone_files(&path)?);
        } else if meta.is_file() && path.extension().and_then(|s| s.to_str()) == Some("stone") {
            paths.push(path);
        }
    }

    Ok(paths)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),

    #[error("reading {path}")]
    StoneRead { source: stone::read::Error, path: PathBuf },

    #[error("writing {path}")]
    StoneWrite { source: stone::write::Error, path: PathBuf },

    #[error("package {0} has two files with the same release {1}")]
    DuplicateRelease(package::Name, u64),

    #[error("meta payload missing")]
    MissingMetaPayload,

    #[error(transparent)]
    MissingMetaField(#[from] MissingMetaFieldError),

    #[error(transparent)]
    StripPrefix(#[from] StripPrefixError),

    #[error("client")]
    Client(#[from] client::Error),

    #[error("non-utf8 path: {path}")]
    NonUtf8Path { path: PathBuf },
}
