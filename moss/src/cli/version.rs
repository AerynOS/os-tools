// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{arg, ArgMatches, Command};

/// Construct the Version command
pub fn command() -> Command {
    Command::new("version")
        .about("Display version and exit")
        .arg(arg!(-f --"full" "Print the full build and version info").action(clap::ArgAction::SetTrue))
}

pub fn handle(args: &ArgMatches) {
    let show_full = args.get_flag("full");
    if show_full {
        print_full();
    } else {
        print();
    }
}

/// Print program version
pub fn print() {
    tracing::info!("moss {}", tools_buildinfo::get_simple_version());
}

/// Print additional build information
pub fn print_full() {
    tracing::info!("moss {}", tools_buildinfo::get_full_version());
}
