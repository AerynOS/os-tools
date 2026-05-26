// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use log::warn;

use crate::{
    Provider, db,
    package::{self, Package},
    repository,
};

#[derive(Debug)]
pub struct Repository {
    active: repository::Cached,
}

impl Repository {
    pub fn new(active: repository::Cached) -> Self {
        Self { active }
    }

    pub fn priority(&self) -> u64 {
        self.active.repository.priority.into()
    }

    pub fn package(&self, id: &package::Id) -> Option<Package> {
        let result = self.active.db.get(id);

        match result {
            Ok(meta) => Some(self.to_package(id.clone(), meta)),
            Err(db::meta::Error::RowNotFound) => None,
            Err(error) => {
                warn!("failed to query repository package: {error}");
                None
            }
        }
    }

    fn to_package(&self, id: package::Id, meta: package::Meta) -> Package {
        Package {
            id,
            meta: package::Meta {
                // TODO: Is there a more type-safe way to do this vs mutation? Can
                // a new type help here?
                uri: meta
                    .uri
                    .and_then(|relative| self.active.repository.uri.join(&relative).ok())
                    .map(|url| url.to_string()),
                origin: Some(self.active.id.to_string()),
                ..meta
            },
            flags: package::Flags::new().with_available(),
        }
    }

    fn query(&self, flags: package::Flags, filter: Option<db::meta::Filter<'_>>) -> Vec<Package> {
        if flags.available || flags == package::Flags::default() {
            // TODO: Error handling
            let packages = match self.active.db.query(filter) {
                Ok(packages) => packages,
                Err(error) => {
                    warn!("failed to query repository packages: {error}");
                    return vec![];
                }
            };

            packages
                .into_iter()
                .map(|(id, meta)| self.to_package(id, meta))
                .collect()
        } else {
            vec![]
        }
    }

    pub fn list(&self, flags: package::Flags) -> Vec<Package> {
        self.query(flags, None)
    }

    pub fn query_keyword(&self, keyword: &str, flags: package::Flags) -> Vec<Package> {
        self.query(flags, Some(db::meta::Filter::Keyword(keyword)))
    }

    /// Query all packages that match the given provider identity
    pub fn query_provider(&self, provider: &Provider, flags: package::Flags) -> Vec<Package> {
        self.query(flags, Some(db::meta::Filter::Provider(provider.clone())))
    }

    pub fn query_name(&self, package_name: &package::Name, flags: package::Flags) -> Vec<Package> {
        self.query(flags, Some(db::meta::Filter::Name(package_name.clone())))
    }

    pub fn query_provider_id_only(&self, provider: &Provider, flags: package::Flags) -> Vec<package::Id> {
        if flags.available || flags == package::Flags::default() {
            // TODO: Error handling
            match self.active.db.provider_packages(provider) {
                Ok(packages) => packages,
                Err(error) => {
                    warn!("failed to query repository packages: {error}");
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }
}

impl PartialEq for Repository {
    fn eq(&self, other: &Self) -> bool {
        self.active.id.eq(&other.active.id)
    }
}

impl Eq for Repository {}

#[cfg(test)]
mod test {
    use super::*;
    use crate::repository::{Id, Priority};
    use url::Url;

    #[test]
    fn test_metadata_population() {
        let db = db::meta::Database::new(":memory:").unwrap();
        let repo_id = Id::new("test-repo");
        let repo_uri = Url::parse("https://example.com/repo/").unwrap();

        let cached = repository::Cached {
            id: repo_id.clone(),
            repository: repository::Repository {
                description: "Test Repo".to_string(),
                uri: repo_uri.clone(),
                priority: Priority::new(10),
                active: true,
            },
            db: db.clone(),
        };

        let plugin = Repository::new(cached);

        let pkg_id = package::Id::from("test-pkg-1.0-1.x86_64");
        let meta = package::Meta {
            name: package::Name::from("test-pkg".to_string()),
            version_identifier: "1.0".to_string(),
            source_release: 1,
            build_release: 1,
            architecture: "x86_64".to_string(),
            summary: "Test package".to_string(),
            description: "Test package description".to_string(),
            source_id: "test-pkg".to_string(),
            homepage: "https://example.com".to_string(),
            licenses: vec!["MIT".to_string()],
            dependencies: Default::default(),
            providers: Default::default(),
            conflicts: Default::default(),
            uri: Some("test-pkg-1.0-1.x86_64.stone".to_string()),
            hash: Some("sha256:hash".to_string()),
            download_size: Some(1024),
            origin: None,
        };

        db.add(pkg_id.clone(), meta).unwrap();

        // Test query_name (which uses query helper)
        let results = plugin.query_name(
            &package::Name::from("test-pkg".to_string()),
            package::Flags::default().with_available(),
        );
        assert_eq!(results.len(), 1);
        let pkg = &results[0];

        assert_eq!(pkg.meta.origin, Some("test-repo".to_string()));
        assert_eq!(
            pkg.meta.uri,
            Some("https://example.com/repo/test-pkg-1.0-1.x86_64.stone".to_string())
        );

        // Test package (by id)
        let pkg_by_id = plugin.package(&pkg_id).unwrap();
        assert_eq!(pkg_by_id.meta.origin, Some("test-repo".to_string()));
        assert_eq!(
            pkg_by_id.meta.uri,
            Some("https://example.com/repo/test-pkg-1.0-1.x86_64.stone".to_string())
        );
    }
}
