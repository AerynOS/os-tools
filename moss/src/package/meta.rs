// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;

use astr::AStr;
use derive_more::{Debug, Display, From, Into};
use stone::{StonePayloadMetaPrimitive, StonePayloadMetaRecord, StonePayloadMetaTag};
use thiserror::Error;

use crate::{Dependency, Provider, dependency};

/// A package identifier constructed from metadata fields
#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd, Display)]
#[debug("{_0:?}")]
pub struct Id(pub(super) AStr);

/// The name of a [`super::Package`]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, From, Into, Display)]
pub struct Name(String);

impl Name {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn contains(&self, text: &str) -> bool {
        self.0.contains(text)
    }
}

/// The metadata of a [`super::Package`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// Package name
    pub name: Name,
    /// Human readable version identifier
    pub version_identifier: String,
    /// Package release as set in stone.yml
    pub source_release: u64,
    /// Build machinery specific build release
    pub build_release: u64,
    /// Architecture this was built for
    pub architecture: String,
    /// Brief one line summary of the package
    pub summary: String,
    /// Description of the package
    pub description: String,
    /// The source-grouping ID
    pub source_id: String,
    /// Where'd we find this guy..
    pub homepage: String,
    /// Licenses this is available under
    pub licenses: Vec<String>,
    /// All dependencies
    pub dependencies: BTreeSet<Dependency>,
    /// All providers, including name()
    pub providers: BTreeSet<Provider>,
    /// All providers that conflict with this package
    pub conflicts: BTreeSet<Provider>,
    /// If relevant: uri to fetch from
    pub uri: Option<String>,
    /// If relevant: hash for the download
    pub hash: Option<String>,
    /// How big is this package in the repo..?
    pub download_size: Option<u64>,
}

impl Meta {
    pub fn from_stone_payload(payload: &[StonePayloadMetaRecord]) -> Result<Self, MissingMetaFieldError> {
        let name = find_meta_string(payload, StonePayloadMetaTag::Name)?;
        let version_identifier = find_meta_string(payload, StonePayloadMetaTag::Version)?;
        let source_release = find_meta_u64(payload, StonePayloadMetaTag::Release)?;
        let build_release = find_meta_u64(payload, StonePayloadMetaTag::BuildRelease)?;
        let architecture = find_meta_string(payload, StonePayloadMetaTag::Architecture)?;
        let summary = find_meta_string(payload, StonePayloadMetaTag::Summary)?;
        let description = find_meta_string(payload, StonePayloadMetaTag::Description)?;
        let source_id = find_meta_string(payload, StonePayloadMetaTag::SourceID)?;
        let homepage = find_meta_string(payload, StonePayloadMetaTag::Homepage)?;
        let uri = find_meta_string(payload, StonePayloadMetaTag::PackageURI).ok();
        let hash = find_meta_string(payload, StonePayloadMetaTag::PackageHash).ok();
        let download_size = find_meta_u64(payload, StonePayloadMetaTag::PackageSize).ok();

        let licenses = payload
            .iter()
            .filter_map(|meta| meta_string(meta, StonePayloadMetaTag::License))
            .collect();
        let dependencies = payload.iter().filter_map(meta_dependency).collect();
        let providers = payload
            .iter()
            .filter_map(meta_provider)
            // Add package name as provider
            .chain(Some(Provider {
                kind: dependency::Kind::PackageName,
                name: name.clone(),
            }))
            .collect();
        let conflicts = payload.iter().filter_map(meta_conflict).collect();

        Ok(Meta {
            name: Name::from(name),
            version_identifier,
            source_release,
            build_release,
            architecture,
            summary,
            description,
            source_id,
            homepage,
            licenses,
            dependencies,
            providers,
            conflicts,
            uri,
            hash,
            download_size,
        })
    }

    pub fn to_stone_payload(self) -> Vec<StonePayloadMetaRecord> {
        vec![
            (
                StonePayloadMetaTag::Name,
                StonePayloadMetaPrimitive::String(self.name.to_string()),
            ),
            (
                StonePayloadMetaTag::Version,
                StonePayloadMetaPrimitive::String(self.version_identifier),
            ),
            (
                StonePayloadMetaTag::Release,
                StonePayloadMetaPrimitive::Uint64(self.source_release),
            ),
            (
                StonePayloadMetaTag::BuildRelease,
                StonePayloadMetaPrimitive::Uint64(self.build_release),
            ),
            (
                StonePayloadMetaTag::Architecture,
                StonePayloadMetaPrimitive::String(self.architecture),
            ),
            (
                StonePayloadMetaTag::Summary,
                StonePayloadMetaPrimitive::String(self.summary),
            ),
            (
                StonePayloadMetaTag::Description,
                StonePayloadMetaPrimitive::String(self.description),
            ),
            (
                StonePayloadMetaTag::SourceID,
                StonePayloadMetaPrimitive::String(self.source_id),
            ),
            (
                StonePayloadMetaTag::Homepage,
                StonePayloadMetaPrimitive::String(self.homepage),
            ),
        ]
        .into_iter()
        .chain(
            self.uri
                .map(|uri| (StonePayloadMetaTag::PackageURI, StonePayloadMetaPrimitive::String(uri))),
        )
        .chain(self.hash.map(|hash| {
            (
                StonePayloadMetaTag::PackageHash,
                StonePayloadMetaPrimitive::String(hash),
            )
        }))
        .chain(self.download_size.map(|size| {
            (
                StonePayloadMetaTag::PackageSize,
                StonePayloadMetaPrimitive::Uint64(size),
            )
        }))
        .chain(
            self.licenses
                .into_iter()
                .map(|license| (StonePayloadMetaTag::License, StonePayloadMetaPrimitive::String(license))),
        )
        .chain(self.dependencies.into_iter().map(|dep| {
            (
                StonePayloadMetaTag::Depends,
                StonePayloadMetaPrimitive::Dependency(dep.kind.into(), dep.name),
            )
        }))
        .chain(
            self.providers
                .into_iter()
                // We re-add this on ingestion / it's implied
                .filter(|provider| provider.kind != dependency::Kind::PackageName)
                .map(|provider| {
                    (
                        StonePayloadMetaTag::Provides,
                        StonePayloadMetaPrimitive::Provider(provider.kind.into(), provider.name),
                    )
                }),
        )
        .chain(
            self.conflicts
                .into_iter()
                // We re-add this on ingestion / it's implied
                .map(|conflict| {
                    (
                        StonePayloadMetaTag::Conflicts,
                        StonePayloadMetaPrimitive::Provider(conflict.kind.into(), conflict.name),
                    )
                }),
        )
        .map(|(tag, kind)| StonePayloadMetaRecord { tag, primitive: kind })
        .collect()
    }

    /// Return a reusable ID
    pub fn id(&self) -> Id {
        Id(format!(
            "{}-{}-{}.{}",
            &self.name.0, &self.version_identifier, &self.source_release, &self.architecture
        )
        .into())
    }
}

fn find_meta_string(
    meta: &[StonePayloadMetaRecord],
    tag: StonePayloadMetaTag,
) -> Result<String, MissingMetaFieldError> {
    meta.iter()
        .find_map(|meta| meta_string(meta, tag))
        .ok_or(MissingMetaFieldError(tag))
}

fn find_meta_u64(meta: &[StonePayloadMetaRecord], tag: StonePayloadMetaTag) -> Result<u64, MissingMetaFieldError> {
    meta.iter()
        .find_map(|meta| meta_u64(meta, tag))
        .ok_or(MissingMetaFieldError(tag))
}

fn meta_u64(meta: &StonePayloadMetaRecord, tag: StonePayloadMetaTag) -> Option<u64> {
    if meta.tag == tag {
        Some(match meta.primitive {
            StonePayloadMetaPrimitive::Int8(i) => i as _,
            StonePayloadMetaPrimitive::Uint8(i) => i as _,
            StonePayloadMetaPrimitive::Int16(i) => i as _,
            StonePayloadMetaPrimitive::Uint16(i) => i as _,
            StonePayloadMetaPrimitive::Int32(i) => i as _,
            StonePayloadMetaPrimitive::Uint32(i) => i as _,
            StonePayloadMetaPrimitive::Int64(i) => i as _,
            StonePayloadMetaPrimitive::Uint64(i) => i,
            _ => return None,
        })
    } else {
        None
    }
}

fn meta_string(meta: &StonePayloadMetaRecord, tag: StonePayloadMetaTag) -> Option<String> {
    match (meta.tag, &meta.primitive) {
        (meta_tag, StonePayloadMetaPrimitive::String(value)) if meta_tag == tag => Some(value.clone()),
        _ => None,
    }
}

fn meta_dependency(meta: &StonePayloadMetaRecord) -> Option<Dependency> {
    if let StonePayloadMetaPrimitive::Dependency(kind, name) = meta.primitive.clone() {
        Some(Dependency {
            kind: dependency::Kind::from_stone_dependency(kind)?,
            name,
        })
    } else {
        None
    }
}

fn meta_provider(meta: &StonePayloadMetaRecord) -> Option<Provider> {
    match (meta.tag, meta.primitive.clone()) {
        (StonePayloadMetaTag::Provides, StonePayloadMetaPrimitive::Provider(kind, name)) => Some(Provider {
            kind: dependency::Kind::from_stone_dependency(kind)?,
            name: name.clone(),
        }),
        _ => None,
    }
}

fn meta_conflict(meta: &StonePayloadMetaRecord) -> Option<Provider> {
    match (meta.tag, meta.primitive.clone()) {
        (StonePayloadMetaTag::Conflicts, StonePayloadMetaPrimitive::Provider(kind, name)) => Some(Provider {
            kind: dependency::Kind::from_stone_dependency(kind)?,
            name: name.clone(),
        }),
        _ => None,
    }
}

#[derive(Debug, Error)]
#[error("Missing metadata field: {0:?}")]
pub struct MissingMetaFieldError(pub StonePayloadMetaTag);
