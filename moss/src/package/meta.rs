// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, sync::Arc};

use derive_more::{AsRef, Debug, Display, From, Into};
use stone::payload;
use thiserror::Error;

use crate::{Dependency, Provider, dependency};

/// A package identifier constructed from metadata fields
#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd, Display)]
#[debug("{_0:?}")]
pub struct Id(pub(super) Arc<str>);

/// The name of a [`super::Package`]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, AsRef, From, Into, Display)]
pub struct Name(String);

impl Name {
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
    pub fn from_stone_payload(payload: &[payload::Meta]) -> Result<Self, MissingMetaFieldError> {
        let name = find_meta_string(payload, payload::meta::Tag::Name)?;
        let version_identifier = find_meta_string(payload, payload::meta::Tag::Version)?;
        let source_release = find_meta_u64(payload, payload::meta::Tag::Release)?;
        let build_release = find_meta_u64(payload, payload::meta::Tag::BuildRelease)?;
        let architecture = find_meta_string(payload, payload::meta::Tag::Architecture)?;
        let summary = find_meta_string(payload, payload::meta::Tag::Summary)?;
        let description = find_meta_string(payload, payload::meta::Tag::Description)?;
        let source_id = find_meta_string(payload, payload::meta::Tag::SourceID)?;
        let homepage = find_meta_string(payload, payload::meta::Tag::Homepage)?;
        let uri = find_meta_string(payload, payload::meta::Tag::PackageURI).ok();
        let hash = find_meta_string(payload, payload::meta::Tag::PackageHash).ok();
        let download_size = find_meta_u64(payload, payload::meta::Tag::PackageSize).ok();

        let licenses = payload
            .iter()
            .filter_map(|meta| meta_string(meta, payload::meta::Tag::License))
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

    pub fn to_stone_payload(self) -> Vec<payload::Meta> {
        use payload::meta::{Kind, Tag};

        vec![
            (Tag::Name, Kind::String(self.name.to_string())),
            (Tag::Version, Kind::String(self.version_identifier)),
            (Tag::Release, Kind::Uint64(self.source_release)),
            (Tag::BuildRelease, Kind::Uint64(self.build_release)),
            (Tag::Architecture, Kind::String(self.architecture)),
            (Tag::Summary, Kind::String(self.summary)),
            (Tag::Description, Kind::String(self.description)),
            (Tag::SourceID, Kind::String(self.source_id)),
            (Tag::Homepage, Kind::String(self.homepage)),
        ]
        .into_iter()
        .chain(self.uri.map(|uri| (Tag::PackageURI, Kind::String(uri))))
        .chain(self.hash.map(|hash| (Tag::PackageHash, Kind::String(hash))))
        .chain(self.download_size.map(|size| (Tag::PackageSize, Kind::Uint64(size))))
        .chain(
            self.licenses
                .into_iter()
                .map(|license| (Tag::License, Kind::String(license))),
        )
        .chain(
            self.dependencies
                .into_iter()
                .map(|dep| (Tag::Depends, Kind::Dependency(dep.kind.into(), dep.name))),
        )
        .chain(
            self.providers
                .into_iter()
                // We re-add this on ingestion / it's implied
                .filter(|provider| provider.kind != dependency::Kind::PackageName)
                .map(|provider| (Tag::Provides, Kind::Provider(provider.kind.into(), provider.name))),
        )
        .chain(
            self.conflicts
                .into_iter()
                // We re-add this on ingestion / it's implied
                .map(|conflict| (Tag::Conflicts, Kind::Provider(conflict.kind.into(), conflict.name))),
        )
        .map(|(tag, kind)| payload::Meta { tag, kind })
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

fn find_meta_string(meta: &[payload::Meta], tag: payload::meta::Tag) -> Result<String, MissingMetaFieldError> {
    meta.iter()
        .find_map(|meta| meta_string(meta, tag))
        .ok_or(MissingMetaFieldError(tag))
}

fn find_meta_u64(meta: &[payload::Meta], tag: payload::meta::Tag) -> Result<u64, MissingMetaFieldError> {
    meta.iter()
        .find_map(|meta| meta_u64(meta, tag))
        .ok_or(MissingMetaFieldError(tag))
}

fn meta_u64(meta: &payload::Meta, tag: payload::meta::Tag) -> Option<u64> {
    if meta.tag == tag {
        Some(match meta.kind {
            payload::meta::Kind::Int8(i) => i as _,
            payload::meta::Kind::Uint8(i) => i as _,
            payload::meta::Kind::Int16(i) => i as _,
            payload::meta::Kind::Uint16(i) => i as _,
            payload::meta::Kind::Int32(i) => i as _,
            payload::meta::Kind::Uint32(i) => i as _,
            payload::meta::Kind::Int64(i) => i as _,
            payload::meta::Kind::Uint64(i) => i,
            _ => return None,
        })
    } else {
        None
    }
}

fn meta_string(meta: &payload::Meta, tag: payload::meta::Tag) -> Option<String> {
    match (meta.tag, &meta.kind) {
        (meta_tag, payload::meta::Kind::String(value)) if meta_tag == tag => Some(value.clone()),
        _ => None,
    }
}

fn meta_dependency(meta: &payload::Meta) -> Option<Dependency> {
    if let payload::meta::Kind::Dependency(kind, name) = meta.kind.clone() {
        Some(Dependency {
            kind: dependency::Kind::from(kind),
            name,
        })
    } else {
        None
    }
}

fn meta_provider(meta: &payload::Meta) -> Option<Provider> {
    match (meta.tag, meta.kind.clone()) {
        (payload::meta::Tag::Provides, payload::meta::Kind::Provider(kind, name)) => Some(Provider {
            kind: dependency::Kind::from(kind),
            name: name.clone(),
        }),
        _ => None,
    }
}

fn meta_conflict(meta: &payload::Meta) -> Option<Provider> {
    match (meta.tag, meta.kind.clone()) {
        (payload::meta::Tag::Conflicts, payload::meta::Kind::Provider(kind, name)) => Some(Provider {
            kind: dependency::Kind::from(kind),
            name: name.clone(),
        }),
        _ => None,
    }
}

#[derive(Debug, Error)]
#[error("Missing metadata field: {0:?}")]
pub struct MissingMetaFieldError(pub payload::meta::Tag);
