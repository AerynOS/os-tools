// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;

use kdl::{FormatConfig, KdlDocument, KdlNode, KdlNodeFormat};

use super::decode::{self, decode_package};
use super::encode::push_child;
use crate::system_model::encode::push_repository_fields;
use crate::{Provider, Repository, repository};

pub fn sync_packages<'a>(
    content: &str,
    packages_to_remove: &BTreeSet<&Provider>,
    packages_to_add: impl Iterator<Item = &'a str>,
) -> Result<String, decode::Error> {
    let mut document: KdlDocument = content.parse().map_err(decode::Error::ParseKdlDocument)?;

    // If we already have packages defined, remove requested packages
    let packages = if let Some(packages) = document.get_mut("packages") {
        if let Some(children) = packages.children_mut() {
            children.nodes_mut().retain(|child| {
                if let Ok(package) = decode_package(child) {
                    !packages_to_remove.contains(&package)
                } else {
                    false
                }
            });
        }

        packages
    }
    // Otherwise default as empty node
    else {
        document.nodes_mut().push(KdlNode::new("packages"));
        document.get_mut("packages").expect("just pushed")
    };

    // Add requested packages
    for (idx, name) in packages_to_add.enumerate() {
        push_child(packages, name, |node| {
            // Add whitespace / comment
            if idx == 0 {
                node.set_format(KdlNodeFormat {
                    leading: "\n    // Added by moss\n    ".to_owned(),
                    terminator: "\n".to_owned(),
                    ..Default::default()
                });
            } else {
                node.autoformat_config(&FormatConfig::builder().indent_level(1).build());
            }
        });
    }

    Ok(document.to_string())
}

pub fn update_repositories<'a>(
    content: &str,
    repositories_to_update: impl Iterator<Item = (&'a repository::Id, &'a Repository)>,
) -> Result<String, decode::Error> {
    let mut document: KdlDocument = content.parse().map_err(decode::Error::ParseKdlDocument)?;

    if let Some(existing_repos) = document.get_mut("repositories")
        && let Some(children) = existing_repos.children_mut()
    {
        for (id, repo) in repositories_to_update {
            if let Some(node) = children.get_mut(id.as_ref()) {
                *node = KdlNode::new(node.name().clone());
                push_repository_fields(node, repo);
                node.autoformat_config(&FormatConfig::builder().indent_level(1).build());
            }
        }
    }

    Ok(document.to_string())
}

#[cfg(test)]
mod test {
    use crate::{Package, package, system_model};

    use super::*;

    const CONTENT: &str = r#"repositories

// My comment
packages {
    // my comment
    a
     b
    delete-me
    "soname(foo.so)"

  // Weird trailing comment / whitespace before closing delim
  }

// Trailing comment
"#;

    #[test]
    fn test_update_remove_all() {
        const EXPECTED: &str = "repositories

// My comment
packages {
  // Weird trailing comment / whitespace before closing delim
  }

// Trailing comment
";

        let system_model = system_model::decode(CONTENT).unwrap();

        let updated = system_model.sync_packages(&[]).unwrap();

        assert_eq!(updated.encoded, EXPECTED);
    }

    #[test]
    fn test_update_add() {
        const EXPECTED: &str = r#"repositories

// My comment
packages {
    // my comment
    a
     b
    delete-me
    "soname(foo.so)"

    // Added by moss
    c
    "soname(asdf.so)"

  // Weird trailing comment / whitespace before closing delim
  }

// Trailing comment
"#;

        let system_model = system_model::decode(CONTENT).unwrap();

        let updated = system_model
            .sync_packages(&[
                // Original
                package("a"),
                package("b"),
                package("delete-me"),
                package(r#"soname(foo.so)"#),
                // Added
                package("c"),
                package(r#"soname(asdf.so)"#),
            ])
            .unwrap();

        assert_eq!(updated.encoded, EXPECTED);
    }

    #[test]
    fn test_update_full() {
        const EXPECTED: &str = r#"repositories

// My comment
packages {
    // my comment
    a
     b
    "soname(foo.so)"

    // Added by moss
    c
    "soname(asdf.so)"

  // Weird trailing comment / whitespace before closing delim
  }

// Trailing comment
"#;

        let system_model = system_model::decode(CONTENT).unwrap();

        let updated = system_model
            .sync_packages(&[
                // Original
                package("a"),
                package("b"),
                package(r#"soname(foo.so)"#),
                // Added
                package("c"),
                package(r#"soname(asdf.so)"#),
            ])
            .unwrap();

        assert_eq!(updated.encoded, EXPECTED);
    }

    fn package(name: &'static str) -> Package {
        Package {
            id: package::Id::from(name),
            meta: package::Meta {
                name: name.to_owned().into(),
                version_identifier: "".to_owned(),
                source_release: 0,
                build_release: 0,
                architecture: "".to_owned(),
                summary: "".to_owned(),
                description: "".to_owned(),
                source_id: "".to_owned(),
                homepage: "".to_owned(),
                licenses: vec![],
                dependencies: Default::default(),
                providers: [Provider::from_name(name).unwrap()].into_iter().collect(),
                conflicts: Default::default(),
                uri: None,
                hash: None,
                download_size: None,
            },
            flags: package::Flags::default(),
        }
    }
}
