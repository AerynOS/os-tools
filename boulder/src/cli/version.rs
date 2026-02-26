// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Print version info and exit")]
pub struct Command {
    #[arg(
        long = "full",
        help = "Print the full build and version info",
        default_value = "false"
    )]
    full: bool,
}

pub fn handle(command: Command) {
    if command.full {
        print_full();
    } else {
        print();
    }
}

/// Print program version
pub fn print() {
    println!("boulder {}", tools_buildinfo::get_simple_version());
}

/// Print additional build information
pub fn print_full() {
    println!("boulder {}", tools_buildinfo::get_full_version());
}
