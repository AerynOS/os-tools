use std::collections::BTreeSet;

use kdl::{FormatConfig, KdlDocument, KdlNode, KdlNodeFormat};

use super::decode::{self, decode_package};
use super::encode::push_child;
use crate::Provider;

pub fn update<'a>(
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

        let updated = system_model.update(&[]).unwrap();

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
            .update(&[
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
            .update(&[
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

    fn package(name: &str) -> Package {
        Package {
            id: package::Id::from(name.to_owned()),
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
