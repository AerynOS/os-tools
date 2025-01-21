// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io::{copy, Read, Seek, SeekFrom},
    os::unix::fs::symlink,
    path::PathBuf,
};

use clap::{arg, ArgMatches, Command};
use fs_err::{self as fs, File};
use moss::package::{self, MissingMetaFieldError};
use stone::{payload::layout, read::PayloadKind};
use thiserror::{self, Error};
use tui::{ProgressBar, ProgressStyle};

pub fn command() -> Command {
    Command::new("extract")
        .about("Extract a `.stone` content to disk")
        .long_about("For all valid content-bearing archives, extract to disk")
        .arg(arg!(<PATH> ... "files to inspect").value_parser(clap::value_parser!(PathBuf)))
}

/// Handle the `extract` command
pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let paths = args
        .get_many::<PathBuf>("PATH")
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();

    // Begin unpack
    fs::create_dir_all(".stoneStore")?;

    let content_store = PathBuf::from(".stoneStore");

    for path in paths {
        println!("Extract: {path:?}");

        let rdr = File::open(path).map_err(Error::IO)?;
        let mut reader = stone::read(rdr).map_err(Error::Format)?;

        let payloads = reader.payloads()?.collect::<Result<Vec<_>, _>>()?;
        let content = payloads.iter().find_map(PayloadKind::content);
        let layouts = payloads.iter().find_map(PayloadKind::layout);
        let meta = payloads.iter().find_map(PayloadKind::meta).ok_or(Error::MissingMeta)?;

        let pkg = package::Meta::from_stone_payload(&meta.body).map_err(Error::MalformedMeta)?;
        let extraction_root = PathBuf::from(pkg.id().to_string());

        // Cleanup old extraction root
        if extraction_root.exists() {
            fs::remove_dir_all(&extraction_root)?;
        }

        if let Some(content) = content {
            let content_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(".stoneContent")?;

            let progress = ProgressBar::new(content.header.plain_size).with_style(
                ProgressStyle::with_template("|{bar:20.cyan/bue}| {percent}%")
                    .unwrap()
                    .progress_chars("■≡=- "),
            );
            reader.unpack_content(content, &mut progress.wrap_write(&content_file))?;

            // Extract all indices from the `.stoneContent` into hash-indexed unique files
            payloads
                .iter()
                .filter_map(PayloadKind::index)
                .flat_map(|p| &p.body)
                .map(|idx| {
                    // Split file reader over index range
                    let mut file = &content_file;
                    file.seek(SeekFrom::Start(idx.start))?;
                    let mut split_file = (&mut file).take(idx.end - idx.start);

                    let mut output = File::create(format!(".stoneStore/{:02x}", idx.digest))?;

                    copy(&mut split_file, &mut output)?;

                    Ok(())
                })
                .collect::<Result<Vec<_>, Error>>()?;

            fs::remove_file(".stoneContent")?;
        }

        if let Some(layouts) = layouts {
            for layout in &layouts.body {
                match &layout.entry {
                    layout::Entry::Regular(id, target) => {
                        let store_path = content_store.join(format!("{id:02x}"));
                        let target_disk = extraction_root.join("usr").join(target);

                        // drop it into a valid dir
                        // TODO: Fix the permissions & mask
                        let directory_target = target_disk.parent().unwrap();
                        fs::create_dir_all(directory_target)?;

                        // link from CA store
                        fs::hard_link(store_path, target_disk)?;
                    }
                    layout::Entry::Symlink(source, target) => {
                        let target_disk = extraction_root.join("usr").join(target);
                        let directory_target = target_disk.parent().unwrap();

                        // ensure dumping ground exists
                        fs::create_dir_all(directory_target)?;

                        // join the link path to the directory target for relative joinery
                        symlink(source, target_disk)?;
                    }
                    layout::Entry::Directory(target) => {
                        let target_disk = extraction_root.join("usr").join(target);
                        // TODO: Fix perms!
                        fs::create_dir_all(target_disk)?;
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    // Clean up.
    fs::remove_dir_all(content_store)?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Missing metadata")]
    MissingMeta,

    #[error("malformed meta")]
    MalformedMeta(#[from] MissingMetaFieldError),

    #[error("io")]
    IO(#[from] std::io::Error),

    #[error("stone format")]
    Format(#[from] stone::read::Error),
}
