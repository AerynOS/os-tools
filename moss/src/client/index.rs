use std::{
    collections::{BTreeMap, btree_map},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf, StripPrefixError},
    time::Duration,
};

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use sha2::{Digest, Sha256};
use stone::{StoneHeaderV1FileType, StoneReadError, StoneWriteError, StoneWriter};
use thiserror::Error;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::{
    client,
    package::{self, Meta, MissingMetaFieldError},
};

/// Index a directory of stone files & produce a `stone.index` index file
///
/// If `output_dir` is `None`, `stone.index` is output to `index_dir`
#[tracing::instrument(skip_all)]
pub fn index(index_dir: &Path, output_dir: Option<&Path>) -> Result<(), Error> {
    let output_dir = output_dir.unwrap_or(index_dir);

    let stone_files = enumerate_stone_files(index_dir)?;

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
        output_dir,
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

    write_index(output_dir, map, &total_progress)?;

    multi_progress.clear()?;

    println!("\nIndex file written to {:?}", output_dir.join("stone.index").display());

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
        let mut writer = StoneWriter::new(&mut file, StoneHeaderV1FileType::Repository)?;

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
    output_dir: &'a Path,
    multi_progress: &'a MultiProgress,
    total_progress: &'a ProgressBar,
}

fn get_meta(path: &Path, ctx: GetMetaCtx<'_>) -> Result<Meta, Error> {
    let relative_path: Utf8PathBuf = rel_path_from_to(ctx.output_dir, path)
        .try_into()
        .map_err(|_| Error::NonUtf8Path { path: path.to_owned() })?;

    let progress = ctx
        .multi_progress
        .insert_before(ctx.total_progress, ProgressBar::new_spinner());
    progress.enable_steady_tick(Duration::from_millis(150));

    let (size, hash) = stat_file(path, &relative_path, &progress)?;

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
        } else if meta.is_file() && is_stone_file(&path)? && path.extension().and_then(|s| s.to_str()) != Some("index")
        {
            paths.push(path);
        }
    }

    Ok(paths)
}

fn is_stone_file(file: &Path) -> io::Result<bool> {
    let mut file = File::open(file)?;

    let mut buffer = [0u8; 4];
    const MAGIC: &[u8; 4] = b"\0mos";

    match file.read_exact(&mut buffer) {
        Ok(()) => {
            if &buffer == MAGIC {
                return Ok(true);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {}
        Err(e) => return Err(e),
    }

    Ok(false)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),

    #[error("reading {path}")]
    StoneRead { source: StoneReadError, path: PathBuf },

    #[error("writing {path}")]
    StoneWrite { source: StoneWriteError, path: PathBuf },

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

/// Make a relative path that points to `to` if the current working directory is `from_dir`.
///
/// Inputs must start with `/` (be absolute) and not contain `.` or `..` segments.
fn rel_path_from_to(from_dir: &Path, to: &Path) -> PathBuf {
    assert!(from_dir.is_absolute());
    assert!(to.is_absolute());

    if from_dir == to {
        return ".".into();
    }

    let mut from_dir = from_dir.to_owned();
    let mut result = PathBuf::new();
    loop {
        if let Ok(suffix) = to.strip_prefix(&from_dir) {
            result.push(suffix);
            return result;
        }

        let popped = from_dir.pop();
        assert!(popped, "strip_prefix must succeed when reaching the root");

        result.push("..");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::rel_path_from_to;

    #[test]
    fn test_rel_path_from_to_strips_prefix() {
        assert_eq!(rel_path_from_to(Path::new("/"), Path::new("/root")), Path::new("root"));
        assert_eq!(rel_path_from_to(Path::new("/x"), Path::new("/x/y/z")), Path::new("y/z"));
    }

    #[test]
    fn test_rel_path_from_to_works_for_identical_inputs() {
        assert_eq!(rel_path_from_to(Path::new("/"), Path::new("/")), Path::new("."));
        assert_eq!(rel_path_from_to(Path::new("/a"), Path::new("/a")), Path::new("."));
        assert_eq!(
            rel_path_from_to(Path::new("/hello/world"), Path::new("/hello/world")),
            Path::new(".")
        );
    }

    #[test]
    fn test_rel_path_from_to_works_for_almost_identical_inputs() {
        assert_eq!(rel_path_from_to(Path::new("/a/"), Path::new("/a")), Path::new("."));
    }

    #[test]
    fn test_rel_path_from_to_goes_up_one_level() {
        assert_eq!(
            rel_path_from_to(Path::new("/a/b"), Path::new("/a/x")),
            Path::new("../x")
        );
    }

    #[test]
    fn test_rel_path_from_to_goes_up_two_levels() {
        assert_eq!(
            rel_path_from_to(Path::new("/a/b/c"), Path::new("/a/x")),
            Path::new("../../x")
        );
    }

    #[test]
    fn test_rel_path_from_to_goes_up_to_root() {
        assert_eq!(rel_path_from_to(Path::new("/a"), Path::new("/b")), Path::new("../b"));
    }
}
