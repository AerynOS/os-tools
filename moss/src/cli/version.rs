// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "version", about = "Display version and exit")]
pub struct Command {
    #[arg(short, long, help = "Print the full build and version info")]
    full: bool,
}

impl Command {
    /// Handles the "version" subcommand.
    pub fn handle(self) {
        if self.full {
            print_full();
        } else {
            print();
        }
    }
}

pub fn print() {
    println!("moss {}", tools_buildinfo::get_simple_version());
}

fn print_full() {
    println!("moss {}", tools_buildinfo::get_full_version());
}
