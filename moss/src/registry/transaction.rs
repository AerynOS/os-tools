// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use dag::Dag;
use thiserror::Error;

use crate::{package, Provider, Registry};

enum ProviderFilter {
    /// Must be installed
    InstalledOnly(Provider),

    /// Filter the lookup to current selection scope
    Selections(Provider),

    // Available in upstream repositories
    Available(Provider),
}

/// Dependency lookup strategy
#[derive(Clone, Copy, Debug, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum Lookup {
    /// Lookup only installed packages
    InstalledOnly,
    /// Lookup installed packages first
    PreferInstalled,
    /// Lookup available packages first
    PreferAvailable,
}

/// A Transaction is used to modify one system state to another
#[derive(Clone, Debug)]
pub struct Transaction<'a> {
    /// Bound to a registry
    registry: &'a Registry,

    /// unique set of package ids
    packages: Dag<package::Id>,

    /// Dependency lookup strategy
    lookup: Lookup,
}

/// Construct a new Transaction wrapped around the underlying [`Registry`].
///
/// At this point the registry is initialised and we can probe the installed
/// set.
pub fn new(registry: &Registry, lookup: Lookup) -> Result<Transaction<'_>, Error> {
    tracing::debug!("creating new transaction");
    Ok(Transaction {
        registry,
        packages: Dag::default(),
        lookup,
    })
}

impl Transaction<'_> {
    /// Remove a set of packages and their reverse dependencies
    pub fn remove(&mut self, packages: Vec<package::Id>) {
        // Get transposed subgraph
        let transposed = self.packages.transpose();
        let subgraph = transposed.subgraph(&packages);

        // For each node, remove it from transaction graph
        for package in subgraph.iter_nodes() {
            // Remove that package
            self.packages.remove_node(package);
        }
    }

    /// Return the package IDs in the fully baked configuration
    pub fn finalize(&self) -> impl Iterator<Item = &package::Id> + '_ {
        self.packages.topo()
    }

    /// Update internal package graph with all incoming packages & their deps
    #[tracing::instrument(skip_all, fields(lookup = %self.lookup))]
    pub fn add(&mut self, incoming: Vec<package::Id>) -> Result<(), Error> {
        let mut items = incoming;

        while !items.is_empty() {
            let mut next = vec![];
            for check_id in items {
                self.add_step(check_id, &mut next)?;
            }
            items = next;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(%check_id, check_name))]
    fn add_step(&mut self, check_id: package::Id, next: &mut Vec<package::Id>) -> Result<(), Error> {
        // Ensure node is added and get its index
        let check_node = self.packages.add_node_or_get_index(&check_id);

        // Grab this package in question
        let package = self.registry.by_id(&check_id).next();
        let package = package.ok_or(Error::NoCandidate(check_id.into()))?;

        tracing::Span::current().record("check_name", package.meta.name.as_ref());
        tracing::debug!(
            num_dependencies = package.meta.dependencies.len(),
            "added package to transaction"
        );

        for dependency in package.meta.dependencies {
            let provider = Provider {
                kind: dependency.kind,
                name: dependency.name,
            };

            // Now get it resolved
            let search_id = self.resolve_provider(provider)?;

            // Add dependency node
            let need_search = !self.packages.node_exists(&search_id);
            let dep_node = self.packages.add_node_or_get_index(&search_id);

            // No dag node for it previously
            if need_search {
                tracing::debug!(?search_id, "adding package to next");
                next.push(search_id);
            }

            // Connect w/ edges (rejects cyclical & duplicate edges)
            self.packages.add_edge(check_node, dep_node);
        }

        Ok(())
    }

    // Try all strategies to resolve a provider for installation
    fn resolve_provider(&self, provider: Provider) -> Result<package::Id, Error> {
        match self.lookup {
            Lookup::InstalledOnly => self
                .resolve_provider_with_filter(ProviderFilter::Selections(provider.clone()))
                .or_else(|_| self.resolve_provider_with_filter(ProviderFilter::InstalledOnly(provider.clone()))),
            Lookup::PreferInstalled => self
                .resolve_provider_with_filter(ProviderFilter::Selections(provider.clone()))
                .or_else(|_| self.resolve_provider_with_filter(ProviderFilter::InstalledOnly(provider.clone())))
                .or_else(|_| self.resolve_provider_with_filter(ProviderFilter::Available(provider.clone()))),
            Lookup::PreferAvailable => self
                .resolve_provider_with_filter(ProviderFilter::Selections(provider.clone()))
                .or_else(|_| self.resolve_provider_with_filter(ProviderFilter::Available(provider.clone())))
                .or_else(|_| self.resolve_provider_with_filter(ProviderFilter::InstalledOnly(provider.clone()))),
        }
    }

    /// Attempt to resolve the filterered provider
    fn resolve_provider_with_filter(&self, filter: ProviderFilter) -> Result<package::Id, Error> {
        match filter {
            ProviderFilter::Available(provider) => self
                .registry
                .by_provider_id_only(&provider, package::Flags::new().with_available())
                .next()
                .ok_or(Error::NoCandidate(provider.to_string())),
            ProviderFilter::InstalledOnly(provider) => self
                .registry
                .by_provider_id_only(&provider, package::Flags::new().with_installed())
                .next()
                .ok_or(Error::NoCandidate(provider.to_string())),
            ProviderFilter::Selections(provider) => self
                .registry
                .by_provider_id_only(&provider, package::Flags::default())
                .find(|id| self.packages.node_exists(id))
                .ok_or(Error::NoCandidate(provider.to_string())),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("No such name: {0}")]
    NoCandidate(String),

    #[error("Not yet implemented")]
    NotImplemented,

    #[error("meta db")]
    Database(#[from] crate::db::meta::Error),
}
