// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};
use itertools::Itertools;
use moss::{
    Installation, Package, Provider,
    client::{self, Client, vfs},
    environment,
};
use stone::StonePayloadLayoutFile;
use thiserror::Error;
use tui::{Styled, TermSize};
use vfs::tree::BlitFile;

const COLUMN_WIDTH: usize = 20;

pub fn command() -> Command {
    Command::new("info")
        .about("Query packages")
        .long_about("List detailed package information from all available sources")
        .arg(arg!(<NAME> ... "Packages to query").value_parser(clap::value_parser!(String)))
        .arg(arg!(-f --files ... "Show files provided by package").action(clap::ArgAction::SetTrue))
}

/// For all arguments, try to match a package
pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let pkgs = args
        .get_many::<String>("NAME")
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    let show_files = args.get_flag("files");

    let client = Client::new(environment::NAME, installation)?;

    for pkg in pkgs {
        let lookup = Provider::from_name(&pkg).unwrap();
        let resolved = client.lookup_packages_by_provider(&lookup);

        if resolved.is_empty() {
            return Err(Error::NotFound(pkg));
        }

        for candidate in resolved {
            print_package(&candidate);

            if candidate.flags.installed && show_files {
                let vfs = vfs([&candidate.id], &client.installation)?;
                print_files(vfs);
            }
            println!();
        }
    }

    Ok(())
}

/// Print the title for each metadata section
fn print_titled(title: &'static str) {
    let display_width = COLUMN_WIDTH - title.len();
    print!("{}{:display_width$} ", title.bold(), " ");
}

/// Print paragraph with breaks
fn print_paragraph(p: &str) {
    let term_width = TermSize::default().width;
    let available_width = term_width - COLUMN_WIDTH;

    // Split into paragraphs by empty lines
    let paragraphs = p.lines().collect::<Vec<_>>();
    let paragraphs = paragraphs
        .split(|line| line.trim().is_empty())
        .filter(|para| !para.is_empty());

    let mut first_paragraph = true;

    for paragraph in paragraphs {
        if !first_paragraph {
            println!(); // Add blank line between paragraphs
        }

        // Join the lines and split into words for wrapping
        let text = paragraph.join(" ");
        let mut current_line = String::new();
        let mut first_line = true;

        for word in text.split_whitespace() {
            if current_line.len() + word.len() < available_width {
                if !current_line.is_empty() {
                    current_line.push(' ');
                }
                current_line.push_str(word);
            } else {
                // Print current line
                if first_line && first_paragraph {
                    println!("{}", current_line.dim());
                    first_line = false;
                } else {
                    println!("{:COLUMN_WIDTH$} {}", " ", current_line.dim());
                }
                current_line = word.to_owned();
            }
        }

        // Print any remaining content
        if !current_line.is_empty() {
            if first_line && first_paragraph {
                println!("{}", current_line.dim());
            } else {
                println!("{:COLUMN_WIDTH$} {}", " ", current_line.dim());
            }
        }

        first_paragraph = false;
    }
}

fn print_list<T>(items: impl IntoIterator<Item = T>)
where
    T: ToString,
{
    for (idx, item) in items.into_iter().enumerate() {
        match idx {
            0 => println!("• {}", item.to_string()),
            _ => println!("{:COLUMN_WIDTH$} • {}", " ", item.to_string()),
        }
    }
}

/// Pretty print a package
fn print_package(pkg: &Package) {
    print_titled("Name");
    println!("{}", pkg.meta.name);
    print_titled("Status");
    if pkg.flags.installed {
        println!("Installed");
    } else {
        println!("Not installed");
    }
    print_titled("Version");
    println!("{}", pkg.meta.version_identifier);
    print_titled("Release number");
    println!("{}", pkg.meta.source_release);
    if pkg.meta.build_release > 1 {
        print_titled("Build Release");
        println!("{}", pkg.meta.build_release);
    }
    print_titled("Homepage");
    println!("{}", pkg.meta.homepage);
    print_titled("Summary");
    println!("{}", pkg.meta.summary);
    print_titled("Description");
    print_paragraph(&pkg.meta.description);
    if !pkg.meta.dependencies.is_empty() {
        println!();
        print_titled("Dependencies");
        print_list(pkg.meta.dependencies.iter().sorted());
    }
    if !pkg.meta.providers.is_empty() {
        println!();
        print_titled("Providers");
        print_list(pkg.meta.providers.iter().sorted());
    }
}

fn print_files(vfs: vfs::Tree<client::PendingFile>) {
    let files = vfs
        .iter()
        .filter_map(|file| {
            if file.kind().is_directory() {
                return None;
            }

            let path = file.path();
            let meta = match &file.layout.file {
                StonePayloadLayoutFile::Regular(hash, _) => Some(format!(" ({hash:2x})")),
                StonePayloadLayoutFile::Symlink(source, _) => Some(format!(" -> {source}")),
                _ => None,
            };

            Some((path, meta))
        })
        .collect::<Vec<_>>();

    if files.is_empty() {
        return;
    }

    print_titled("Files");
    println!();
    for (path, meta) in files {
        println!("  {path}{}", meta.unwrap_or_default().dim());
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("No such package {0}")]
    NotFound(String),
    #[error("client")]
    Client(#[from] client::Error),
}
