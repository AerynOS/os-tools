// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgMatches, Command};

use moss::client;
use moss::dependency;
use moss::package::{self, Name};
use moss::{Client, Installation, Provider, environment};
use tui::Styled;
use tui::pretty::{ColumnDisplay, print_columns};

const ARG_KEYWORD: &str = "KEYWORD";
const FLAG_INSTALLED: &str = "installed";
const FLAG_PROVIDES: &str = "provides";

/// Returns the Clap struct for this command.
pub fn command() -> Command {
    Command::new("search")
        .visible_alias("sr")
        .about("Search packages")
        .long_about("Search packages by looking into package names and summaries.")
        .arg(
            Arg::new(ARG_KEYWORD)
                .required(true)
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new(FLAG_INSTALLED)
                .short('i')
                .long("installed")
                .num_args(0)
                .help("Search among installed packages only"),
        )
        .arg(
            Arg::new(FLAG_PROVIDES)
                .short('p')
                .long("provides")
                .num_args(0)
                .help("Search for packages providing a binary"),
        )
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keyword = args.get_one::<String>(ARG_KEYWORD).unwrap();
    let only_installed = args.get_flag(FLAG_INSTALLED);
    let provides = args.get_flag(FLAG_PROVIDES);

    let client = Client::new(environment::NAME, installation)?;
    let flags = if only_installed {
        package::Flags::new().with_installed()
    } else {
        package::Flags::new().with_available()
    };

    let output = if provides {
        search_providing_packages(client, flags, keyword)
    } else {
        search_packages(client, flags, keyword)
    };

    if output.is_empty() {
        return Ok(());
    }

    print_columns(&output, 1);

    Ok(())
}

fn search_packages(client: Client, flags: package::Flags, keyword: &str) -> Vec<Output> {
    client
        .search_packages(keyword, flags)
        .map(|pkg| Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
        })
        .collect()
}

fn search_providing_packages(client: Client, flags: package::Flags, name: &str) -> Vec<Output> {
    // We need to search both Binary and SystemBinary for possible programs
    // TODO: Could include shared libraries down the line, maybe with a flag
    [dependency::Kind::Binary, dependency::Kind::SystemBinary]
        .into_iter()
        .flat_map(|kind| {
            let provider = Provider {
                kind,
                name: name.to_owned(),
            };
            client.lookup_packages_by_provider(&provider, flags)
        })
        .map(|pkg| Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
}

struct Output {
    name: Name,
    summary: String,
}

impl ColumnDisplay for Output {
    fn get_display_width(&self) -> usize {
        self.name.as_str().chars().count()
    }

    fn display_column(&self, writer: &mut impl std::io::prelude::Write, _col: tui::pretty::Column, width: usize) {
        let _ = write!(
            writer,
            "{}{:width$}  {}",
            self.name.as_str().bold(),
            " ".repeat(width),
            self.summary
        );
    }
}
