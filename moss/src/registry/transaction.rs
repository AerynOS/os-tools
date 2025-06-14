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

    /// Use a pinned package if one exists
    /// for this provider
    Pinned(Provider),

    // Look beyond installed/selections
    All(Provider),
}

#[derive(Clone, Copy, Debug)]
enum Lookup {
    InstalledOnly,
    Global,
}

/// A Transaction is used to modify one system state to another
#[derive(Clone, Debug)]
pub struct Transaction<'a> {
    // Bound to a registry
    registry: &'a Registry,

    // unique set of package ids
    packages: Dag<package::Id>,

    /// packages which are always considered first
    /// during [`ProviderFilter::Pinned`] but
    /// aren't part of `packages` DAG
    pinned_providers: Vec<package::Id>,
}

/// Construct a new Transaction wrapped around the underlying [`Registry`].
///
/// At this point the registry is initialised and we can probe the installed
/// set.
pub(super) fn new(registry: &Registry) -> Result<Transaction<'_>, Error> {
    tracing::debug!("creating new transaction");
    Ok(Transaction {
        registry,
        packages: Dag::default(),
        pinned_providers: vec![],
    })
}

/// Populate the transaction on initialisation
pub(super) fn new_with_installed(registry: &Registry, incoming: Vec<package::Id>) -> Result<Transaction<'_>, Error> {
    let mut tx = new(registry)?;
    tx.update(incoming, Lookup::InstalledOnly)?;
    Ok(tx)
}

impl Transaction<'_> {
    /// Add a package to this transaction
    pub fn add(&mut self, incoming: Vec<package::Id>) -> Result<(), Error> {
        self.update(incoming, Lookup::Global)
    }

    /// Pins to the provided packages if a provider lookup matches one of these
    pub fn pin_providers(&mut self, packages: impl IntoIterator<Item = package::Id>) {
        self.pinned_providers.extend(packages.into_iter().inspect(|pkg_id| {
            tracing::debug!(?pkg_id, "pinning package");
        }));
    }

    /// Remove a set of packages and their reverse dependencies
    pub fn remove(&mut self, packages: Vec<package::Id>) {
        // Get transposed subgraph
        let transposed = self.packages.transpose();
        let subgraph = transposed.subgraph(&packages);

        // For each node, remove it from transaction graph
        subgraph.iter_nodes().for_each(|package| {
            // Remove that package
            self.packages.remove_node(package);
        });
    }

    /// Return the package IDs in the fully baked configuration
    pub fn finalize(&self) -> impl Iterator<Item = &package::Id> + '_ {
        self.packages.topo()
    }

    /// Update internal package graph with all incoming packages & their deps
    #[tracing::instrument(skip_all, fields(?lookup))]
    fn update(&mut self, incoming: Vec<package::Id>, lookup: Lookup) -> Result<(), Error> {
        let mut items = incoming;

        while !items.is_empty() {
            let mut next = vec![];
            for check_id in items {
                self.update_step(check_id, &mut next, lookup)?;
            }
            items = next;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(?check_id, check_name))]
    fn update_step(&mut self, check_id: package::Id, next: &mut Vec<package::Id>, lookup: Lookup) -> Result<(), Error> {
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
            let search_id = match lookup {
                Lookup::Global => self.resolve_installation_provider(provider)?,
                Lookup::InstalledOnly => self.resolve_provider(ProviderFilter::InstalledOnly(provider))?,
            };

            // Add dependency node
            let need_search = !self.packages.node_exists(&search_id);
            let dep_node = self.packages.add_node_or_get_index(&search_id);

            // No dag node for it previously
            if need_search {
                tracing::debug!(?search_id, "adding package to next");
                next.push(search_id.clone());
            }

            // Connect w/ edges (rejects cyclical & duplicate edges)
            self.packages.add_edge(check_node, dep_node);
        }

        Ok(())
    }

    /// Attempt to resolve the filterered provider
    fn resolve_provider(&self, filter: ProviderFilter) -> Result<package::Id, Error> {
        match filter {
            ProviderFilter::All(provider) => self
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
            ProviderFilter::Pinned(provider) => self
                .registry
                .by_provider_id_only(&provider, package::Flags::default())
                .find(|id| self.pinned_providers.contains(id))
                .ok_or(Error::NoCandidate(provider.to_string())),
        }
    }

    // Try all strategies to resolve a provider for installation
    fn resolve_installation_provider(&self, provider: Provider) -> Result<package::Id, Error> {
        self.resolve_provider(ProviderFilter::Pinned(provider.clone()))
            .or_else(|_| self.resolve_provider(ProviderFilter::Selections(provider.clone())))
            .or_else(|_| self.resolve_provider(ProviderFilter::InstalledOnly(provider.clone())))
            .or_else(|_| self.resolve_provider(ProviderFilter::All(provider)))
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
