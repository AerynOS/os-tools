// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{arg, ArgMatches, Command};
use std::fs::File;
use std::path::PathBuf;
use stone::payload::layout;
use stone::payload::meta;
use stone::read::PayloadKind;
use thiserror::Error;

const COLUMN_WIDTH: usize = 20;

pub fn command() -> Command {
    Command::new("inspect")
        .about("Examine raw stone files")
        .long_about("Show detailed (debug) information on a local `.stone` file")
        .arg(arg!(<PATH> ... "files to inspect").value_parser(clap::value_parser!(PathBuf)))
}

///
/// Inspect the given .stone files and print results
///
pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let paths = args
        .get_many::<PathBuf>("PATH")
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();

    // Process each input path in order.
    for path in paths {
        let mut file = File::open(&path)?;
        let mut reader = stone::read(&mut file)?;

        let header = reader.header;
        let payloads = reader.payloads()?;

        // Grab the header version
        print!("{path:?} = stone container version {:?}", header.version());

        for payload in payloads.flatten() {
            let mut layouts = vec![];

            // Grab deps/providers/conflicts
            let mut deps = vec![];
            let mut provs = vec![];
            let mut cnfls = vec![];

            match payload {
                PayloadKind::Layout(l) => layouts = l.body,
                PayloadKind::Meta(meta) => {
                    println!();

                    for record in meta.body {
                        let name = format!("{:?}", record.tag);

                        match &record.kind {
                            meta::Kind::Provider(k, p) if record.tag == meta::Tag::Provides => {
                                provs.push(format!("{}({})", k, p))
                            }
                            meta::Kind::Provider(k, p) if record.tag == meta::Tag::Conflicts => {
                                cnfls.push(format!("{}({})", k, p))
                            }
                            meta::Kind::Dependency(k, d) => deps.push(format!("{}({})", k, d)),
                            meta::Kind::String(s) => {
                                println!("{:width$} : {}", name, s, width = COLUMN_WIDTH)
                            }
                            meta::Kind::Int64(i) => {
                                println!("{:width$} : {}", name, i, width = COLUMN_WIDTH)
                            }
                            meta::Kind::Uint64(i) => {
                                println!("{:width$} : {}", name, i, width = COLUMN_WIDTH)
                            }
                            _ => {
                                println!("{:width$} : {:?}", name, record, width = COLUMN_WIDTH)
                            }
                        }
                    }
                }
                _ => {}
            }

            if !deps.is_empty() {
                println!("\n{:width$} :", "Dependencies", width = COLUMN_WIDTH);
                for dep in deps {
                    println!("    - {dep}");
                }
            }
            if !provs.is_empty() {
                println!("\n{:width$} :", "Providers", width = COLUMN_WIDTH);
                for prov in provs {
                    println!("    - {prov}");
                }
            }
            if !cnfls.is_empty() {
                println!("\n{:width$} :", "Conflicts", width = COLUMN_WIDTH);
                for cnfl in cnfls {
                    println!("    - {cnfl}");
                }
            }

            if !layouts.is_empty() {
                println!("\n{:width$} :", "Layout entries", width = COLUMN_WIDTH);
                for layout in layouts {
                    match layout.entry {
                        layout::Entry::Regular(hash, target) => {
                            println!("    - /usr/{} - [Regular] {:032x}", target, hash)
                        }
                        layout::Entry::Directory(target) => {
                            println!("    - /usr/{} [Directory]", target)
                        }
                        layout::Entry::Symlink(source, target) => {
                            println!("    - /usr/{} -> {} [Symlink]", target, source)
                        }
                        _ => unreachable!(),
                    };
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    IO(#[from] std::io::Error),

    #[error("stone format")]
    Format(#[from] stone::read::Error),
}
