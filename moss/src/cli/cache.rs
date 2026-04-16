// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{io, path::PathBuf};

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use fs_err as fs;
use moss::{
    Client, Installation,
    client::{self},
    environment,
    package::{self},
    repository, runtime, util,
};
use stone::{StoneDecodedPayload, StonePayloadMetaPrimitive, StonePayloadMetaTag};
use thiserror::Error;
use url::Url;

pub fn command() -> clap::Command {
    Command::command()
}

#[derive(Debug, Parser)]
#[command(name = "cache", about = "Managed cached data")]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    #[command(
        about = "Prune cached artefacts",
        long_about = "Prune cached artefacts

This will remove all downloaded stones & unpacked asset data for packages not in any state or active repository."
    )]
    Prune,
    #[command(
        about = "Seed stones in index URI to folder",
        long_about = "Seed stones in specified index URI to folder"
    )]
    Seed {
        #[arg(help = "index uri")]
        uri: String,
        #[arg(help = "output directory")]
        output_dir: PathBuf,
    },
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let command = Command::from_arg_matches(args).expect("validated by clap");

    match command.subcommand {
        Subcommand::Prune => handle_prune(installation),
        Subcommand::Seed { uri, output_dir } => handle_seed(uri, &output_dir),
    }
}

fn handle_seed(uri: String, output_dir: &PathBuf) -> Result<(), Error> {
    util::ensure_dir_exists(output_dir)?;

    let out_index_path = output_dir.join("stone.index");

    let parsed_uri = Url::parse(&uri)?;

    runtime::block_on(repository::fetch_index(parsed_uri.clone(), &out_index_path))?;

    let mut file = fs::File::open(&out_index_path)?;
    let mut reader = stone::read(&mut file)?;

    let payloads = reader.payloads()?;

    let mut package_metas = Vec::new();

    for payload in payloads.flatten() {
        if let StoneDecodedPayload::Meta(mut meta) = payload {
            for record in &mut meta.body {
                // Fix up the meta.url manually
                if record.tag == StonePayloadMetaTag::PackageURI
                    && let StonePayloadMetaPrimitive::String(s) = &mut record.primitive
                {
                    *s = parsed_uri.join(s)?.to_string();
                }
            }

            let pkg_meta = package::Meta::from_stone_payload(&meta.body)?;
            package_metas.push(pkg_meta);
        }
    }

    client::fetch::fetch(&package_metas, output_dir, false)?;

    Ok(())
}

fn handle_prune(installation: Installation) -> Result<(), Error> {
    let client = Client::new(environment::NAME, installation).map_err(Error::SetupClient)?;

    let num_removed_files = client.prune_cache().map_err(Error::PruneCache)?;

    if num_removed_files > 0 {
        let s = if num_removed_files > 1 { "s" } else { "" };

        println!("{num_removed_files} file{s} removed");
    } else {
        println!("No files to remove");
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to setup moss client")]
    SetupClient(#[source] client::Error),

    #[error("failed to prune cache")]
    PruneCache(#[source] client::Error),

    #[error("url parse error")]
    Url(#[from] url::ParseError),

    #[error("fetch index file")]
    FetchIndex(#[from] repository::FetchError),

    #[error("read index file")]
    ReadStone(#[from] stone::StoneReadError),

    #[error("failed to fetch package")]
    FetchError(#[from] client::fetch::Error),

    #[error("malformed meta")]
    MalformedMeta(#[from] package::MissingMetaFieldError),

    #[error("io")]
    Io(#[from] io::Error),
}
