// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;

use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgMatches, Command};

use moss::client;
use moss::dependency;
use moss::package::{self, Name};
use moss::{Client, Installation, Provider, environment};
use strum::Display;
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
                .num_args(0..=1)
                .require_equals(true)
                .default_missing_value("binary")
                .value_parser([
                    "library",
                    "name",
                    "soname",
                    "pkgconfig",
                    "interpreter",
                    "cmake",
                    "python",
                    "binary",
                    "sysbinary",
                    "pkgconfig32",
                ])
                .help("Search for packages by provider"),
        )
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Display)]
#[strum(serialize_all = "lowercase")]
enum MatchKind {
    Name,
    Summary,
}

fn map_aliases(value: &str) -> &str {
    match value {
        "library" => "soname",
        _ => value,
    }
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keyword = args.get_one::<String>(ARG_KEYWORD).unwrap();
    let only_installed = args.get_flag(FLAG_INSTALLED);
    let provides = args.get_one::<String>(FLAG_PROVIDES).map(|s| s.as_str());

    let client = Client::new(environment::NAME, installation)?;
    let flags = if only_installed {
        package::Flags::new().with_installed()
    } else {
        package::Flags::new().with_available()
    };

    let mut output = if let Some(provider) = provides {
        provides_package(&client, flags, provider, keyword)
    } else {
        search_packages(&client, flags, keyword)
    }?;

    if output.values().all(|pkgs| pkgs.is_empty()) {
        return Ok(());
    }

    for (match_kind, value) in output.iter_mut() {
        println!("Matched field: {match_kind}");
        value.sort();
        print_columns(value, 1);
    }

    Ok(())
}

fn search_packages(
    client: &Client,
    flags: package::Flags,
    keyword: &str,
) -> Result<BTreeMap<MatchKind, Vec<Output>>, Error> {
    let provider = Provider::from_name(keyword).map_err(|_| Error::ParseError(keyword.to_owned()))?;

    let mut output_kind: BTreeMap<MatchKind, Vec<Output>> = BTreeMap::new();

    match provider.kind {
        dependency::Kind::PackageName => {
            for pkg in client.search_packages(&provider.name, flags) {
                let pkg_name_lowercase = pkg.meta.name.as_str().to_ascii_lowercase();
                let match_kind = if pkg_name_lowercase.contains(&keyword.to_ascii_lowercase()) {
                    MatchKind::Name
                } else {
                    MatchKind::Summary
                };
                output_kind.entry(match_kind).or_default().push(Output {
                    name: pkg.meta.name,
                    summary: pkg.meta.summary,
                    search_match: Some(keyword.to_owned()),
                });
            }
            Ok(output_kind)
        }
        _ => {
            output_kind.insert(
                MatchKind::Name,
                client
                    .lookup_packages_by_provider(&provider, flags)
                    .into_iter()
                    .map(Output::from)
                    .collect(),
            );
            Ok(output_kind)
        }
    }
}

fn provides_package_by_kind(
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

fn provides_package(
    client: &Client,
    flags: package::Flags,
    provider: &str,
    name: &str,
) -> Result<BTreeMap<MatchKind, Vec<Output>>, Error> {
    let mut result: BTreeMap<MatchKind, Vec<Output>> = BTreeMap::new();
    let packages = match provider.parse::<dependency::Kind>() {
        Ok(kind) => provides_package_by_kind(client, flags, name, kind),
        Err(_) => match provider {
            // Default search with no argument to `--provides`
            "binaries" => [dependency::Kind::Binary, dependency::Kind::SystemBinary]
                .into_iter()
                .flat_map(|kind| provides_package_by_kind(client, flags, name, kind))
                .collect(),
            "library" => provides_package_by_kind(client, flags, name, dependency::Kind::SharedLibrary),
            _ => unreachable!("clap restricts valid arguments"),
        },
    };
    result.insert(MatchKind::Name, packages.into_iter().map(Output::from).collect());
    Ok(result)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("Invalid dependency type: {0}")]
    ParseError(String),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Output {
    name: Name,
    summary: String,
    search_match: Option<String>,
}

fn highlight_string(content: &str, expression: &str) -> (String, String, String) {
    if let Some(index) = content.to_ascii_lowercase().find(&expression.to_ascii_lowercase()) {
        let (prefix, body) = content.split_at(index);
        let (matched, suffix) = body.split_at(expression.len());
        (prefix.to_owned(), matched.to_owned(), suffix.to_owned())
    } else {
        (content.to_owned(), String::default(), String::default())
    }
}

impl ColumnDisplay for Output {
    fn get_display_width(&self) -> usize {
        self.name.as_str().chars().count()
    }

    fn display_column(&self, writer: &mut impl std::io::prelude::Write, _col: tui::pretty::Column, width: usize) {
        if let Some(expression) = self.search_match.as_deref() {
            let (name_prefix, name_matched, name_suffix) = highlight_string(self.name.as_str(), expression);
            let (summary_prefix, summary_matched, summary_suffix) = highlight_string(&self.summary, expression);
            let _ = write!(
                writer,
                " {}{}{}{:width$}  {}{}{}",
                name_prefix.bold(),
                name_matched.bold().green(),
                name_suffix.bold(),
                " ".repeat(width),
                summary_prefix.bold(),
                summary_matched.bold().green(),
                summary_suffix.bold(),
            );
        } else {
            let _ = write!(
                writer,
                " {}{:width$}  {}",
                self.name.as_str().bold(),
                " ".repeat(width),
                self.summary
            );
        }
    }
}

impl From<package::Package> for Output {
    fn from(pkg: package::Package) -> Self {
        Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
            search_match: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::LazyLock;

    use super::*;

    static TEST_CLIENT: LazyLock<Option<Client>> = LazyLock::new(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../aosroot");
        let installation = Installation::open(root, None).ok()?;
        Client::new("TEST", installation).ok()
    });

    macro_rules! test_client {
        () => {
            match TEST_CLIENT.as_ref() {
                Some(client) => client,
                None => {
                    eprintln!("Test Skipped: aosroot directory unavailable");
                    return;
                }
            }
        };
    }

    #[test]
    fn test_find_packages() {
        let client = test_client!();
        let flags = package::Flags::new().with_available();
        let output = search_packages(client, flags, "jq").unwrap();
        assert!(!output.is_empty(), "expected match for package jq");
    }

    // #[test]
    // fn test_find_binaries_with_provides_flag() {
    //     let client = test_client!();
    //     let flags = package::Flags::new().with_available();
    //     for binary_name in ["telnet", "toast", "zramctl"] {
    //         // These binary names don't appear when searching by package name
    //         let output = search_packages(client, flags, binary_name).unwrap();
    //         assert!(
    //             output.is_empty(),
    //             "`search {binary_name}` output is not empty: {output:?}"
    //         );
    //
    //         // We can find hits for all these binaries with the `--provides` flag
    //         let output = provides_package(client, flags, "binaries", binary_name).unwrap();
    //         assert!(
    //             !output.is_empty(),
    //             "`search --provides {binary_name} should not be empty"
    //         );
    //     }
    // }

    #[test]
    fn test_find_binaries_with_provider_syntax() {
        let client = test_client!();
        let flags = package::Flags::new().with_available();
        for binary_name in ["telnet", "toast"] {
            // These binary names don't appear when searching by package name
            let output = search_packages(client, flags, binary_name).unwrap();
            assert!(
                output.is_empty(),
                "`search {binary_name}` output is not empty: {output:?}"
            );

            // We can find hits for all these binaries with the provider syntax
            let provider_syntax = format!("binary({binary_name})");
            let output = search_packages(client, flags, &provider_syntax).unwrap();
            assert!(
                !output.is_empty(),
                "`search {provider_syntax}` output should not be empty"
            );
        }
    }

    // #[test]
    // fn test_provider_syntax_produces_same_output_as_provides_flag() {
    //     let client = test_client!();
    //     let flags = package::Flags::new().with_available();
    //     for binary_name in ["hx", "telnet", "toast"] {
    //         let output_a = provides_package(client, flags, "binaries", binary_name).unwrap();
    //         let provider_syntax = format!("binary({binary_name})");
    //         let output_b = search_packages(client, flags, &provider_syntax).unwrap();
    //         assert_eq!(output_a, output_b);
    //     }
    // }
}
