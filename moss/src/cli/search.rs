// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;

use clap::builder::NonEmptyStringValueParser;
use clap::{Arg, ArgMatches, Command};

use moss::client;
use moss::dependency;
use moss::package::{self, Name};
use moss::{Client, Installation, Provider, environment};
use stone::StonePayloadLayoutFile;
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

fn determine_provider(keyword: &str, provides_flag: Option<dependency::Kind>) -> Result<Provider, Error> {
    let mut provider = Provider::from_name(keyword).map_err(|_| Error::Parse(keyword.to_owned()))?;
    if let Some(kind) = provides_flag {
        provider.kind = kind;
    }
    Ok(provider)
}

fn query_packages(client: &Client, flags: package::Flags, provider: Provider) -> BTreeMap<MatchKind, Vec<Output>> {
    match provider {
        Provider {
            kind: dependency::Kind::PackageName,
            name,
        } => search_packages(client, flags, &name),
        _ => search_by_provider(client, flags, provider),
    }
}

fn search_file(mut keyword: String, client: Client) -> Result<(), Error> {
    // moss db doesn't record the /usr/ prefix so strip any combination of it
    // so queries like r/bin/nano, /bin/nano and /usr/bin/nano still succeed.
    let prefix = "/usr/";
    for i in 0..=prefix.len() {
        let suffix = &prefix[i..];
        if keyword.starts_with(suffix) {
            keyword.drain(..suffix.len());
            break;
        }
    }

    let layouts = client.list_layouts()?;

    layouts.into_iter().for_each(|(id, layout)| match layout.file {
        StonePayloadLayoutFile::Regular(_, file)
        | StonePayloadLayoutFile::Symlink(_, file)
        | StonePayloadLayoutFile::Directory(file) => {
            if file.contains(&keyword)
                && let Ok(pkg) = client.resolve_package(&id)
            {
                let name = pkg.meta.name;
                println!("{prefix}{file} from {}", name.as_str().bold());
            }
        }
        _ => {}
    });

    Ok(())
}

pub fn handle(args: &ArgMatches, installation: Installation) -> Result<(), Error> {
    let keyword = args.get_one::<String>(ARG_KEYWORD).unwrap();
    let provides_flag = args
        .get_one::<String>(FLAG_PROVIDES)
        .map(|s| map_aliases(s))
        .map(|s| s.parse::<dependency::Kind>().expect("clap should restrict input"));
    let only_installed = args.get_flag(FLAG_INSTALLED);
    let client = Client::new(environment::NAME, installation)?;

    if keyword.contains('/') {
        return search_file(keyword.to_owned(), client);
    }

    let provider = determine_provider(keyword, provides_flag)?;

    let flags = if only_installed {
        package::Flags::new().with_installed()
    } else {
        package::Flags::new().with_available()
    };

    let output = query_packages(&client, flags, provider);

    if output.values().all(Vec::is_empty) {
        return Ok(());
    }

    for mut value in output.into_values() {
        value.sort();
        print_columns(&value, 1);
    }

    Ok(())
}

fn search_packages(client: &Client, flags: package::Flags, keyword: &str) -> BTreeMap<MatchKind, Vec<Output>> {
    let mut results: BTreeMap<MatchKind, Vec<Output>> = BTreeMap::new();

    let keyword_lowercase = keyword.to_ascii_lowercase();
    for pkg in client.search_packages(keyword, flags) {
        let pkg_name_lowercase = pkg.meta.name.as_str().to_ascii_lowercase();
        let match_kind = if pkg_name_lowercase.contains(&keyword_lowercase) {
            MatchKind::Name
        } else {
            MatchKind::Summary
        };
        results.entry(match_kind).or_default().push(Output {
            name: pkg.meta.name,
            summary: pkg.meta.summary,
            search_match: Some(keyword.to_owned()),
        });
    }
    results
}

fn search_by_provider(client: &Client, flags: package::Flags, provider: Provider) -> BTreeMap<MatchKind, Vec<Output>> {
    let packages = client.lookup_packages_by_provider(&provider, flags);
    BTreeMap::from([(MatchKind::Name, packages.into_iter().map(Output::from).collect())])
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("Invalid dependency type: {0}")]
    Parse(String),

    #[error("db")]
    DB(#[from] moss::db::Error),
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
                "{}{}{}{:width$}  {}{}{}",
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
                "{}{:width$}  {}",
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

    use moss::Registry;
    use moss::package::{self, Name, Package};
    use moss::registry::plugin;
    use std::collections::BTreeSet;
    use std::sync::LazyLock;

    use super::*;

    fn pkg(name: &str, summary: &str, providers: &[Provider]) -> Package {
        let mut providers: BTreeSet<Provider> = providers.iter().cloned().collect();
        providers.insert(provider(dependency::Kind::PackageName, name));
        Package {
            id: package::Id::from(name.to_owned()),
            meta: package::Meta {
                name: Name::from(name.to_owned()),
                summary: summary.to_owned(),
                providers,
                version_identifier: Default::default(),
                source_release: Default::default(),
                build_release: Default::default(),
                architecture: Default::default(),
                description: Default::default(),
                source_id: Default::default(),
                homepage: Default::default(),
                licenses: Default::default(),
                dependencies: Default::default(),
                conflicts: Default::default(),
                uri: Default::default(),
                hash: Default::default(),
                download_size: Default::default(),
            },
            flags: package::Flags::new().with_available(),
        }
    }

    fn provider(kind: dependency::Kind, name: &str) -> Provider {
        Provider {
            kind,
            name: name.to_owned(),
        }
    }

    /// Build a test registry populated with real packages from the AerynOS recipes.
    /// Package names, summaries, and provider metadata are sourced from stone.yaml
    /// and manifest.x86_64.jsonc files in the recipes repository.
    fn test_registry() -> Registry {
        let mut registry = Registry::default();
        let binary = dependency::Kind::Binary;
        let soname = dependency::Kind::SharedLibrary;
        let pkgconfig = dependency::Kind::PkgConfig;

        let packages = vec![
            pkg("jq", "Command-line JSON processor", &[provider(binary, "jq")]),
            pkg("ripgrep", "Recursive text search utility", &[provider(binary, "rg")]),
            pkg(
                "nano",
                "GNU Text Editor",
                &[provider(binary, "nano"), provider(binary, "rnano")],
            ),
            pkg("helix", "A post-modern text editor", &[provider(binary, "hx")]),
            pkg("bash", "GNU Bourne-Again Shell", &[provider(binary, "bash")]),
            pkg(
                "zsh",
                "Extensible shell designed for interactive use",
                &[provider(binary, "zsh")],
            ),
            pkg("fish", "The friendly interactive shell", &[provider(binary, "fish")]),
            pkg(
                "libyaml",
                "YAML 1.1 library",
                &[provider(soname, "libyaml-0.so.2(x86_64)")],
            ),
            pkg(
                "libyaml-devel",
                "Development files for libyaml",
                &[provider(pkgconfig, "yaml-0.1")],
            ),
        ];

        registry.add_plugin(plugin::Plugin::Test(plugin::Test::new(1, packages)));
        registry
    }

    struct TestFixture {
        _root: tempfile::TempDir,
        client: Client,
    }

    static TEST_FIXTURE: LazyLock<TestFixture> = LazyLock::new(|| {
        let root = tempfile::tempdir().unwrap();
        let installation = Installation::open(root.path(), None).unwrap();
        let registry = test_registry();
        let client = Client::mocked(installation, registry).unwrap();
        TestFixture { _root: root, client }
    });

    fn client() -> &'static Client {
        &TEST_FIXTURE.client
    }

    fn flags_available() -> package::Flags {
        package::Flags::default().with_available()
    }

    fn collect_result_names(results: &BTreeMap<MatchKind, Vec<Output>>) -> Vec<String> {
        let mut names: Vec<String> = results
            .values()
            .flat_map(|outputs| outputs.iter().map(|output| output.name.as_str().to_owned()))
            .collect();
        names.sort();
        names
    }

    fn moss(args: &str) -> ArgMatches {
        command().get_matches_from(args.split_whitespace())
    }

    /// Test helper function that approximates the behavior of `handle()`
    fn test_handle(query: &str) -> BTreeMap<MatchKind, Vec<Output>> {
        let args = moss(query);
        let keyword = args.get_one::<String>(ARG_KEYWORD).unwrap();
        let provides_flag = args
            .get_one::<String>(FLAG_PROVIDES)
            .map(|s| map_aliases(s))
            .map(|s| s.parse::<dependency::Kind>().expect("clap should restrict input"));
        let provider = determine_provider(keyword, provides_flag).unwrap();
        query_packages(client(), flags_available(), provider)
    }

    #[test]
    fn test_keyword_exact_name() {
        let output = test_handle("search jq");
        let names = collect_result_names(&output);
        assert_eq!(names, vec!["jq"]);
    }

    #[test]
    fn test_keyword_shell_matches_case_insensitively() {
        // Keyword search is case-insensitive so all three casings return the same results
        for keyword in ["shell", "Shell", "SHELL"] {
            let output = test_handle(&format!("search {keyword}"));
            let names = collect_result_names(&output);
            assert_eq!(names, vec!["bash", "fish", "zsh"]);
        }
    }

    #[test]
    fn test_keyword_summary_match() {
        // "json" appears in jq's summary but not its name
        let output = test_handle("search json");
        let names = collect_result_names(&output);
        assert_eq!(names, vec!["jq"]);
    }

    #[test]
    fn test_keyword_text_matches_multiple() {
        // "text" matches:
        //   helix ("text editor")
        //   nano ("GNU Text Editor")
        //   ripgrep ("text search")
        let output = test_handle("search text");
        let names = collect_result_names(&output);
        assert_eq!(names, vec!["helix", "nano", "ripgrep"]);
    }

    /// TODO: `moss search` could eventually provide results on binary names by default
    #[test]
    fn test_keyword_binary_name_returns_nothing() {
        for query in ["search hx", "search rg"] {
            let output = test_handle(query);
            assert!(output.values().all(Vec::is_empty));
        }
    }

    #[test]
    fn test_keyword_nonexistent_returns_nothing() {
        let output = test_handle("search no-such-package");
        assert!(output.values().all(Vec::is_empty));
    }

    #[test]
    fn test_keyword_uppercase_matches_case_insensitively() {
        let output = test_handle("search NANO");
        let names = collect_result_names(&output);
        assert_eq!(names, vec!["nano"]);
    }

    #[test]
    fn test_provider_binary_hx_finds_helix() {
        let output_provides_flag = test_handle("search --provides=binary hx");
        let output_dependency_syntax = test_handle("search binary(hx)");

        let names_provides_flag = collect_result_names(&output_provides_flag);
        let names_dependency_syntax = collect_result_names(&output_dependency_syntax);

        assert_eq!(names_provides_flag, names_dependency_syntax);
        assert_eq!(names_provides_flag, vec!["helix"]);
    }

    #[test]
    fn test_provider_binary_rg_finds_ripgrep() {
        let output_provides_flag = test_handle("search --provides=binary rg");
        let output_dependency_syntax = test_handle("search binary(rg)");

        let names_provides_flag = collect_result_names(&output_provides_flag);
        let names_dependency_syntax = collect_result_names(&output_dependency_syntax);

        assert_eq!(names_provides_flag, names_dependency_syntax);
        assert_eq!(names_provides_flag, vec!["ripgrep"]);
    }

    #[test]
    fn test_provider_binary_jq_finds_jq() {
        let output_provides_flag = test_handle("search --provides=binary jq");
        let output_dependency_syntax = test_handle("search binary(jq)");

        let names_provides_flag = collect_result_names(&output_provides_flag);
        let names_dependency_syntax = collect_result_names(&output_dependency_syntax);

        assert_eq!(names_provides_flag, names_dependency_syntax);
        assert_eq!(names_provides_flag, vec!["jq"]);
    }

    #[test]
    fn test_provider_soname_finds_libyaml() {
        let output_provides_flag = test_handle("search --provides=soname libyaml-0.so.2(x86_64)");
        let output_dependency_syntax = test_handle("search soname(libyaml-0.so.2(x86_64))");

        let names_provides_flag = collect_result_names(&output_provides_flag);
        let names_dependency_syntax = collect_result_names(&output_dependency_syntax);

        assert_eq!(names_provides_flag, names_dependency_syntax);
        assert_eq!(names_provides_flag, vec!["libyaml"]);
    }

    #[test]
    fn test_provider_pkgconfig_finds_libyaml_devel() {
        let output_provides_flag = test_handle("search --provides=pkgconfig yaml-0.1");
        let output_dependency_syntax = test_handle("search pkgconfig(yaml-0.1)");

        let names_provides_flag = collect_result_names(&output_provides_flag);
        let names_dependency_syntax = collect_result_names(&output_dependency_syntax);

        assert_eq!(names_provides_flag, names_dependency_syntax);
        assert_eq!(names_provides_flag, vec!["libyaml-devel"]);
    }
}
