// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use crate::{Provider, Repository, repository};

pub fn encode<'a>(
    repositories: impl IntoIterator<Item = (&'a repository::Id, &'a Repository)>,
    packages: impl IntoIterator<Item = &'a Provider>,
) -> String {
    let mut doc = KdlDocument::new();

    doc.nodes_mut().push(encode_repositories(repositories));
    doc.nodes_mut().push(encode_packages(packages));

    doc.autoformat();

    doc.to_string()
}

fn encode_repositories<'a>(repositories: impl IntoIterator<Item = (&'a repository::Id, &'a Repository)>) -> KdlNode {
    let mut node = KdlNode::new("repositories");

    for (id, repo) in repositories {
        push_child(&mut node, id, |repo_node| {
            push_child(repo_node, "description", |description| {
                push_value(description, repo.description.clone());
            });

            match &repo.source {
                repository::Source::DirectIndex(uri) => {
                    push_child(repo_node, "uri", |uri_node| {
                        push_value(uri_node, uri.to_string());
                    });
                }
                repository::Source::RootIndex(repository::RootIndexSource {
                    base_uri,
                    channel,
                    version,
                }) => {
                    push_child(repo_node, "base-uri", |uri_node| {
                        push_value(uri_node, base_uri.to_string());
                    });

                    if channel.as_ref() != repository::DEFAULT_CHANNEL {
                        push_child(repo_node, "channel", |channel_node| {
                            push_value(channel_node, channel.to_string());
                        });
                    }

                    push_child(repo_node, "version", |version_node| {
                        push_value(version_node, version.to_string());
                    });
                }
            }

            push_child(repo_node, "priority", |priority| {
                push_value(priority, i128::from(u64::from(repo.priority)));
            });

            if !repo.active {
                push_child(repo_node, "enabled", |enabled| {
                    push_value(enabled, false);
                });
            }
        });
    }

    node
}

fn encode_packages<'a>(packages: impl IntoIterator<Item = &'a Provider>) -> KdlNode {
    let mut node = KdlNode::new("packages");

    for package in packages {
        push_child(&mut node, package.to_name(), |_| {});
    }

    node
}

pub(super) fn push_child(node: &mut KdlNode, name: impl ToString, f: impl FnOnce(&mut KdlNode)) {
    let mut child = KdlNode::new(name.to_string());

    f(&mut child);

    node.ensure_children().nodes_mut().push(child);
}

pub(super) fn push_value(node: &mut KdlNode, value: impl Into<KdlValue>) {
    node.entries_mut().push(KdlEntry::new(value));
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use crate::Repository;

    use super::*;

    #[test]
    fn test_encode_empty() {
        let expected = "repositories\npackages\n";

        let encoded = encode([], []);

        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_encode() {
        let expected = r#"repositories {
    bar {
        description testing
        base-uri "https://test.dev/"
        channel testing
        version "tag/test-this"
        priority 10
    }
    disabled {
        description disabled
        uri "https://test2.dev/index.stone"
        priority 2
        enabled #false
    }
    foo {
        description test
        base-uri "https://test.dev/"
        version "stream/unstable"
        priority 1
    }
}
packages {
    abc
    xyz
    "pkgconfig(abc)"
    "soname(abc.so)"
    "soname(abc.so.1)"
}
"#;

        let repos = repository::Map::from_iter([
            (
                repository::Id::new("bar"),
                Repository {
                    description: "testing".to_owned(),
                    source: repository::Source::RootIndex(repository::RootIndexSource {
                        base_uri: "https://test.dev".parse().unwrap(),
                        channel: "testing".try_into().unwrap(),
                        version: "tag/test-this".parse().unwrap(),
                    }),
                    priority: repository::Priority::new(10),
                    active: true,
                },
            ),
            (
                repository::Id::new("disabled"),
                Repository {
                    description: "disabled".to_owned(),
                    source: repository::Source::DirectIndex("https://test2.dev/index.stone".parse().unwrap()),
                    priority: repository::Priority::new(2),
                    active: false,
                },
            ),
            (
                repository::Id::new("foo"),
                Repository {
                    description: "test".to_owned(),
                    source: repository::Source::RootIndex(repository::RootIndexSource {
                        base_uri: "https://test.dev".parse().unwrap(),
                        channel: "main".try_into().unwrap(),
                        version: "stream/unstable".parse().unwrap(),
                    }),
                    priority: repository::Priority::new(1),
                    active: true,
                },
            ),
        ]);
        let packages = BTreeSet::from_iter(
            ["abc", "soname(abc.so)", "soname(abc.so.1)", "pkgconfig(abc)", "xyz"]
                .into_iter()
                .map(|s| Provider::from_name(s).unwrap()),
        );

        let encoded = encode(&repos, &packages);

        assert_eq!(encoded, expected);
    }
}
