// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use clap::builder::NonEmptyStringValueParser;

use moss::client::{self};
use moss::{Installation, client::Client, environment};
use stone::StonePayloadLayoutFile;
use tui::Styled;

#[derive(Debug, Parser)]
#[command(
    name = "search-file",
    visible_alias = "sf",
    about = "Search files by looking into installed package"
)]
pub struct Command {
    #[arg(value_parser = NonEmptyStringValueParser::new(), help = "Name of a file or directory to look for")]
    keyword: String,
}

impl Command {
    /// Handles the "search" command.
    pub fn handle(mut self, installation: Installation) -> Result<(), Error> {
        // moss db doesn't record the /usr/ prefix so strip any combination of it
        // so queries like r/bin/nano, /bin/nano and /usr/bin/nano still succeed.
        const PREFIX: &str = "/usr/";
        for i in 0..=PREFIX.len() {
            let suffix = &PREFIX[i..];
            if self.keyword.starts_with(suffix) {
                self.keyword.drain(..suffix.len());
                break;
            }
        }

        let client = Client::new(environment::NAME, installation)?;

        let layouts = client.list_layouts()?;

        layouts.into_iter().for_each(|(id, layout)| match layout.file {
            StonePayloadLayoutFile::Regular(_, file)
            | StonePayloadLayoutFile::Symlink(_, file)
            | StonePayloadLayoutFile::Directory(file) => {
                if file.contains(&self.keyword)
                    && let Ok(pkg) = client.resolve_package(&id)
                {
                    let name = pkg.meta.name;
                    println!("{PREFIX}{file} from {}", name.as_str().bold());
                }
            }
            _ => {}
        });

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
    #[error("db")]
    DB(#[from] moss::db::Error),
}
