// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};
use itertools::Itertools;
use thiserror::Error;

use moss::{
    Installation,
    client::{self, Client},
    environment,
    package::Flags,
};
use tui::Styled;

pub fn command() -> Command {
    Command::new("list")
        .about("List packages")
        .long_about("List packages according to a filter")
        .subcommand_required(true)
        .subcommand(
            Command::new("installed")
                .about("List all installed packages")
                .visible_alias("li")
                .arg(arg!(-e --"explicit" "List explicit packages only")),
        )
        .subcommand(
            Command::new("available")
                .about("List all available packages")
                .visible_alias("la"),
        )
}

/// Handle listing by filter
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let filter_flags = match args.subcommand() {
        Some(("available", _)) => Flags::new().with_available(),
        Some(("installed", args)) => {
            if *args.get_one::<bool>("explicit").unwrap() {
                Flags::new().with_installed().with_explicit()
            } else {
                Flags::new().with_installed()
            }
        }
        _ => unreachable!(),
    };

    // Grab a client for the target, enumerate packages
    let client = Client::new(environment::NAME, installation)?;
    let pkgs = client.list_packages(filter_flags).collect::<Vec<_>>();

    if pkgs.is_empty() {
        return Err(Error::NoneFound);
    }

    // map to renderable state
    let mut set = pkgs
        .into_iter()
        .map(|p| Format {
            name: p.meta.name.to_string(),
            revision: Revision {
                version: p.meta.version_identifier,
                release: p.meta.source_release.to_string(),
            },
            summary: p.meta.summary,
            explicit: if filter_flags == Flags::new().with_installed() {
                p.flags.explicit
            } else {
                true
            },
        })
        .collect_vec();

    // Thanks to priorities, first in list is the winning candidate in list available.
    // Therefore sort by name and dedupe is safe as we mask the lower priority items out.
    set.sort_by_key(|s| s.name.clone());
    set.dedup_by_key(|s| s.name.clone());

    // Grab maximum length
    let max_length = set.iter().map(Format::size).max().unwrap_or_default() + 2;

    // render
    for item in set {
        let width = max_length - item.size() + 2;
        let name = if item.explicit {
            item.name.bold()
        } else {
            item.name.dim()
        };
        print!("{name} {:width$} ", " ");

        let print_revision = |rev: Revision| {
            let version = rev.version.magenta();
            print!("{version}-{}", rev.release.dim());
        };

        // Print revision
        print_revision(item.revision);

        println!(" - {}", item.summary);
    }

    Ok(())
}

#[derive(Debug)]
struct Format {
    name: String,
    summary: String,
    revision: Revision,
    explicit: bool,
}

impl Format {
    fn size(&self) -> usize {
        self.name.len() + self.revision.size()
    }
}

#[derive(Debug)]
struct Revision {
    version: String,
    release: String,
}

impl Revision {
    fn size(&self) -> usize {
        self.version.len() + self.release.len()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("No packages found")]
    NoneFound,
    #[error("client")]
    Client(#[from] client::Error),
}
