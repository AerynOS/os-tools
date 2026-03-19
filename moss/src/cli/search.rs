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
        search_providing_packages(&client, flags, keyword)
    } else {
        search_packages(&client, flags, keyword)
    };

    if output.is_empty() {
        return Ok(());
    }

    print_columns(&output, 1);

    Ok(())
}

fn search_packages(client: &Client, flags: package::Flags, keyword: &str) -> Vec<Output> {
    let provider = Provider::from_name(keyword).expect("Invalid format");

    match provider.kind {
        dependency::Kind::PackageName => client
            .search_packages(&provider.name, flags)
            .map(Output::from)
            .collect(),
        _ => client
            .lookup_packages_by_provider(&provider, flags)
            .into_iter()
            .map(Output::from)
            .collect(),
    }
}

fn search_providing_packages_by_kind(
    client: &Client,
    flags: package::Flags,
    name: &str,
    kind: dependency::Kind,
) -> Vec<package::Package> {
    let provider = Provider {
        kind,
        name: name.to_owned(),
    };
    client.lookup_packages_by_provider(&provider, flags)
}

fn search_providing_packages(client: &Client, flags: package::Flags, name: &str) -> Vec<Output> {
    // We need to search both Binary and SystemBinary for possible programs
    // TODO: Could include shared libraries down the line, maybe with a flag
    [dependency::Kind::Binary, dependency::Kind::SystemBinary]
        .into_iter()
        .flat_map(|kind| search_providing_packages_by_kind(client, flags, name, kind))
        .map(Output::from)
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),
}

#[cfg_attr(test, derive(Debug, PartialEq))] // Only derive these traits in test suite
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

impl From<package::Package> for Output {
    fn from(pkg: package::Package) -> Self {
        Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::LazyLock;

    use super::*;

    static TEST_CLIENT: LazyLock<Client> = LazyLock::new(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../aosroot");
        let installation = Installation::open(root, None).expect("Could not find root");
        Client::new("TEST", installation).expect("Could not set up client")
    });

    #[test]
    fn test_find_packages() {
        let client = &TEST_CLIENT;
        let flags = package::Flags::new().with_available();
        let output = search_packages(client, flags, "jq");
        assert!(!output.is_empty(), "expected match for package jq");
    }

    #[test]
    fn test_find_binaries_with_provides_flag() {
        let client = &TEST_CLIENT;
        let flags = package::Flags::new().with_available();
        for binary_name in ["hx", "telnet", "toast", "zramctl"] {
            // These binary names don't appear when searching by package name
            let output = search_packages(client, flags, binary_name);
            assert!(output.is_empty());

            // We can find hits for all these binaries with the `--provides` flag
            let output = search_providing_packages(client, flags, binary_name);
            assert!(!output.is_empty());
        }
    }

    #[test]
    fn test_find_binaries_with_provider_syntax() {
        let client = &TEST_CLIENT;
        let flags = package::Flags::new().with_available();
        for binary_name in ["hx", "telnet", "toast"] {
            // These binary names don't appear when searching by package name
            let output = search_packages(client, flags, binary_name);
            assert!(output.is_empty());

            // We can find hits for all these binaries with the provider syntax
            let provider_syntax = format!("binary({binary_name})");
            let output = search_packages(client, flags, &provider_syntax);
            assert!(!output.is_empty());
        }
    }

    #[test]
    fn test_provider_syntax_produces_same_output_as_provides_flag() {
        let client = &TEST_CLIENT;
        let flags = package::Flags::new().with_available();
        for binary_name in ["hx", "telnet", "toast"] {
            let output_a = search_providing_packages(client, flags, binary_name);
            let provider_syntax = format!("binary({binary_name})");
            let output_b = search_packages(client, flags, &provider_syntax);
            assert_eq!(output_a, output_b);
        }
    }
}
