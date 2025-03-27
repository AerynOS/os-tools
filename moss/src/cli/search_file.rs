// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Write};

use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgMatches, Command};

use moss::client::{self};
use moss::{Installation, client::Client, environment};
use tui::Styled;

const ARG_KEYWORD: &str = "KEYWORD";

/// Returns the Clap struct for this command.
pub fn command() -> Command {
    Command::new("search-file")
        .visible_alias("sf")
        .about("Search files")
        .long_about("Search files by looking into installed package files.")
        .arg(
            Arg::new(ARG_KEYWORD)
                .required(true)
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keyword = args.get_one::<String>(ARG_KEYWORD).unwrap();

    let client = Client::new(environment::NAME, installation)?;

    let layouts = client.layout_db.all()?;

    layouts.into_iter().for_each(|(id, layout)| match layout.entry {
        stone::payload::layout::Entry::Regular(_, file)
        | stone::payload::layout::Entry::Symlink(_, file)
        | stone::payload::layout::Entry::Directory(file) => {
            if file.contains(keyword) {
                let resolved = client.registry.by_id(&id).next();
                if let Some(pkg) = resolved {
                    let name = pkg.meta.name;
                    println!("/usr/{} from {}", file, name.to_string().bold());
                }
            }
        }
        _ => {}
    });

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
    #[error("db")]
    DB(#[from] moss::db::Error),
}
