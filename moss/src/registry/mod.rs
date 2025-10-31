// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Defines an encapsulation of "query plugins", including an interface
//! for managing and using them.

use itertools::Itertools;

use crate::Provider;
use crate::package::{self, Package};

pub use self::plugin::Plugin;
pub use self::transaction::Transaction;

pub mod plugin;
pub mod transaction;

/// A registry is composed of multiple "query plugins" that
/// provide [`Package`] information
#[derive(Debug, Default)]
pub struct Registry {
    /// Ordered set of plugins
    plugins: Vec<Plugin>,
}

impl Registry {
    /// Add a [`Plugin`] to the [`Registry`]
    pub fn add_plugin(&mut self, plugin: Plugin) {
        self.plugins.push(plugin);
    }

    fn query<'a, T, I>(&'a self, query: impl Fn(&'a Plugin) -> I + Copy + 'a) -> impl Iterator<Item = T> + 'a
    where
        I: IntoIterator<Item = T> + 'a,
    {
        self.plugins
            .iter()
            .sorted_by(|a, b| a.priority().cmp(&b.priority()).reverse())
            .flat_map(query)
    }

    /// Return a sorted stream of [`Package`] by provider
    pub fn by_provider<'a>(
        &'a self,
        provider: &'a Provider,
        flags: package::Flags,
    ) -> impl Iterator<Item = Package> + 'a {
        self.query(move |plugin| plugin.query_provider(provider, flags))
    }

    /// Optimized version of `by_provider` returning [`package::Id`] only
    pub fn by_provider_id_only<'a>(
        &'a self,
        provider: &'a Provider,
        flags: package::Flags,
    ) -> impl Iterator<Item = package::Id> + 'a {
        self.query(move |plugin| plugin.query_provider_id_only(provider, flags))
    }

    /// Return a sorted stream of [`Package`] by name
    pub fn by_name<'a>(
        &'a self,
        package_name: &'a package::Name,
        flags: package::Flags,
    ) -> impl Iterator<Item = Package> + 'a {
        self.query(move |plugin| plugin.query_name(package_name, flags))
    }

    /// Return a sorted stream of [`Package`] by id
    pub fn by_id<'a>(&'a self, id: &'a package::Id) -> impl Iterator<Item = Package> + 'a {
        self.query(move |plugin| plugin.package(id))
    }

    pub fn by_keyword<'a>(&'a self, keyword: &'a str, flags: package::Flags) -> impl Iterator<Item = Package> + 'a {
        self.query(move |plugin| plugin.query_keyword(keyword, flags))
    }

    /// Return a sorted stream of [`Package`] matching the given [`Flags`]
    ///
    /// [`Flags`]: package::Flags
    pub fn list(&self, flags: package::Flags) -> impl Iterator<Item = Package> + '_ {
        self.query(move |plugin| plugin.list(flags))
    }

    /// Return a sorted stream of installed [`Package`]
    pub fn list_installed(&self, flags: package::Flags) -> impl Iterator<Item = Package> + '_ {
        self.list(flags.with_installed())
    }

    /// Return a sorted stream of available [`Package`]
    pub fn list_available(&self, flags: package::Flags) -> impl Iterator<Item = Package> + '_ {
        self.list(flags.with_available())
    }

    /// Return a new transaction for this registry
    pub fn transaction(&self, lookup: transaction::Lookup) -> Result<Transaction<'_>, transaction::Error> {
        transaction::new(self, lookup)
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn test_ordering() {
        let mut registry = Registry::default();

        let package = |id: &str, release| Package {
            id: package::Id::from(id),
            meta: package::Meta {
                name: package::Name::from(id.to_owned()),
                version_identifier: Default::default(),
                source_release: release,
                build_release: Default::default(),
                architecture: Default::default(),
                summary: Default::default(),
                description: Default::default(),
                source_id: Default::default(),
                homepage: Default::default(),
                licenses: Default::default(),
                dependencies: Default::default(),
                providers: Default::default(),
                conflicts: Default::default(),
                uri: Default::default(),
                hash: Default::default(),
                download_size: Default::default(),
            },
            flags: package::Flags::default(),
        };

        registry.add_plugin(Plugin::Test(plugin::Test::new(
            // Priority
            1,
            // Id / release number
            vec![package("a", 0), package("b", 100)],
        )));

        registry.add_plugin(Plugin::Test(plugin::Test::new(
            50,
            vec![package("c", 50), package("d", 1)],
        )));

        let query = registry.list(package::Flags::default());

        // Packages are sorted by plugin priority, desc -> release number, desc
        for (idx, package) in query.enumerate() {
            let id = |id: &str| package::Id::from(id);

            match idx {
                0 => assert_eq!(package.id, id("c")),
                1 => assert_eq!(package.id, id("d")),
                2 => assert_eq!(package.id, id("b")),
                3 => assert_eq!(package.id, id("a")),
                _ => {}
            }
        }
    }

    #[test]
    fn test_flags() {
        let mut registry = Registry::default();

        let package = |id: &str, flags| Package {
            id: package::Id::from(id),
            meta: package::Meta {
                name: package::Name::from(id.to_owned()),
                version_identifier: Default::default(),
                source_release: Default::default(),
                build_release: Default::default(),
                architecture: Default::default(),
                summary: Default::default(),
                description: Default::default(),
                source_id: Default::default(),
                homepage: Default::default(),
                licenses: Default::default(),
                dependencies: Default::default(),
                providers: Default::default(),
                conflicts: Default::default(),
                uri: Default::default(),
                hash: Default::default(),
                download_size: Default::default(),
            },
            flags,
        };

        registry.add_plugin(Plugin::Test(plugin::test::Test::new(
            1,
            vec![
                package("a", package::Flags::new().with_installed()),
                package("b", package::Flags::new().with_available()),
                package("c", package::Flags::new().with_source()),
                package("d", package::Flags::new().with_source().with_installed()),
                package("e", package::Flags::new().with_source().with_available()),
            ],
        )));

        let installed = registry.list_installed(package::Flags::default()).collect();
        let available = registry.list_available(package::Flags::default()).collect();
        let installed_source = registry.list_installed(package::Flags::new().with_source()).collect();
        let available_source = registry.list_available(package::Flags::new().with_source()).collect();

        fn matches(actual: Vec<Package>, expected: &[&'static str]) -> bool {
            let actual = actual
                .into_iter()
                .map(|p| String::from(p.meta.name))
                .collect::<BTreeSet<_>>();
            let expected = expected.iter().map(|&s| s.to_owned()).collect::<BTreeSet<_>>();

            actual == expected
        }

        assert!(matches(installed, &["a", "d"]));
        assert!(matches(available, &["b", "e"]));
        assert!(matches(installed_source, &["d"]));
        assert!(matches(available_source, &["e"]));
    }
}
