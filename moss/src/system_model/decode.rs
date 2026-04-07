// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use kdl::{KdlDocument, KdlNode, KdlValue};
use thiserror::Error;

use crate::{Provider, Repository, SystemModel, dependency, repository};

pub fn decode(content: &str) -> Result<SystemModel, Error> {
    let document: KdlDocument = content.parse().map_err(Error::ParseKdlDocument)?;

    let disable_warning = document
        .get_arg("disable_warning")
        .map(|value| {
            value
                .as_bool()
                .ok_or(Error::InvalidRootValue("disable_warning", "bool", value.to_string()))
        })
        .transpose()?;

    let packages = document
        .get("packages")
        .map(|node| node.iter_children().map(decode_package).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();

    let repositories = document
        .get("repositories")
        .map(|node| node.iter_children().map(decode_repository).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();

    Ok(SystemModel {
        disable_warning: disable_warning.unwrap_or_default(),
        repositories,
        packages,
        encoded: content.to_owned(),
    })
}

pub(super) fn decode_package(node: &KdlNode) -> Result<Provider, Error> {
    Provider::from_name(node.name().value()).map_err(Error::ParseProvider)
}

fn decode_repository(node: &KdlNode) -> Result<(repository::Id, Repository), Error> {
    let name = node.name().value();
    let id = repository::Id::new(name);

    let description = get_child_value(node, "description")
        .map(|value| {
            value.as_string().ok_or(Error::InvalidNodeValue(
                "repository",
                name.to_owned(),
                "description",
                "string",
                value.to_string(),
            ))
        })
        .transpose()?
        .unwrap_or_default();
    let uri = get_child_value(node, "uri")
        .map(|value| {
            value.as_string().ok_or(Error::InvalidNodeValue(
                "repository",
                name.to_owned(),
                "uri",
                "string",
                value.to_string(),
            ))
        })
        .transpose()?
        .ok_or(Error::MissingValue("uri", "repository", name.to_owned()))?
        .parse()
        .map_err(|err| Error::ParseRepositoryUri(err, name.to_owned()))?;
    let enabled = get_child_value(node, "enabled")
        .map(|value| {
            value.as_bool().ok_or(Error::InvalidNodeValue(
                "repository",
                name.to_owned(),
                "uri",
                "bool",
                value.to_string(),
            ))
        })
        .transpose()?
        .unwrap_or(true);
    let priority = get_child_value(node, "priority")
        .map(|value| {
            let int = value.as_integer().ok_or(Error::InvalidNodeValue(
                "repository",
                name.to_owned(),
                "priority",
                "integer",
                value.to_string(),
            ))?;

            u64::try_from(int)
                .map(repository::Priority::new)
                .map_err(|err| Error::ParseRepositoryPriority(err, name.to_owned()))
        })
        .ok_or(Error::MissingValue("priority", "repository", name.to_owned()))??;

    Ok((
        id,
        Repository {
            description: description.to_owned(),
            uri,
            priority,
            active: enabled,
        },
    ))
}

fn get_child_value<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlValue> {
    node.children()
        .and_then(|child| child.get(name))
        .and_then(|node| node.get(0))
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid value for {0}, expected {1} got {2}")]
    InvalidRootValue(&'static str, &'static str, String),
    #[error("invalid value for {0} {1} {2}, expected {3} got {4}")]
    InvalidNodeValue(&'static str, String, &'static str, &'static str, String),
    #[error("missing {0} for {1} {2}")]
    MissingValue(&'static str, &'static str, String),
    #[error("parse as kdl document")]
    ParseKdlDocument(#[source] kdl::KdlError),
    #[error("parse package as provider")]
    ParseProvider(#[source] dependency::ParseError),
    #[error("parse uri for repository {1}")]
    ParseRepositoryUri(#[source] url::ParseError, String),
    #[error("parse priority for repository {1}")]
    ParseRepositoryPriority(#[source] std::num::TryFromIntError, String),
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_decode_kdl() {
        let content = r#"
            repositories {
                volatile {
                    description "where the build infra lands freshly built packages"
                    uri "https://infratest.aerynos.dev/vessel/volatile/x86_64/stone.index"
                    priority 10
                }
                local {
                    description "my locally built packages"
                    uri "file:///path/to/my/stone.index"
                    priority 1
                    enabled #false
                }
            }
            packages {
                foo
                bar-test {
                    why "foo"
                }
                "binary(cc)"
                "pkgconfig(zlib)" {
                    why "bar"
                }
            }
        "#;

        let system_model = decode(content).expect("decode from kdl");

        dbg!(&system_model);
    }
}
