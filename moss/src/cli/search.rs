// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{Arg, ArgMatches, Command, arg, value_parser};

use moss::client;
use moss::package::{self, Name};
use moss::{Client, Installation, environment};
use tui::Styled;
use tui::pretty::{ColumnDisplay, print_columns};

const FLAG_INSTALLED: &str = "installed";

/// Returns the Clap struct for this command.
pub fn command() -> Command {
    Command::new("search")
        .visible_alias("sr")
        .about("Search packages")
        .long_about("Search packages by looking into package names and summaries.")
        .arg(arg!(<KEYWORD> ... "filter search by keywords").value_parser(value_parser!(String)))
        .arg(
            Arg::new(FLAG_INSTALLED)
                .short('i')
                .long("installed")
                .num_args(0)
                .help("Search among installed packages only"),
        )
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keywords = args
        .get_many::<String>("KEYWORD")
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();

    let only_installed = args.get_flag(FLAG_INSTALLED);

    let client = Client::new(environment::NAME, installation)?;
    let flags = if only_installed {
        package::Flags::new().with_installed()
    } else {
        package::Flags::new().with_available()
    };

    let output: Vec<Output> = client
        .search_packages(&keywords, flags)
        .map(|pkg| Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
        })
        .collect();
    print_columns(&output, 1);

    Ok(())
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
