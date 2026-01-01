// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! System trigger management facilities

use std::collections::{BTreeMap, BTreeSet};

use format::Trigger;
use thiserror::Error;

pub mod format;

/// Grouped management of a set of triggers
#[derive(Debug)]
pub struct Collection<'a> {
    handlers: Vec<ExtractedHandler>,
    triggers: BTreeMap<String, &'a Trigger>,
    hits: BTreeMap<String, BTreeSet<format::CompiledHandler>>,
}

#[derive(Debug)]
struct ExtractedHandler {
    id: String,
    pattern: fnmatch::Pattern,
    handler: format::Handler,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing handler reference in {0}: {1}")]
    MissingHandler(String, String),
}

impl<'a> Collection<'a> {
    /// Create a new [Collection] using the given triggers
    pub fn new(triggers: impl IntoIterator<Item = &'a Trigger>) -> Result<Self, Error> {
        let mut handlers = vec![];
        let mut trigger_set = BTreeMap::new();
        for trigger in triggers.into_iter() {
            trigger_set.insert(trigger.name.clone(), trigger);
            for (p, def) in trigger.paths.iter() {
                for used_handler in def.handlers.iter() {
                    // Ensure we have a corresponding handler
                    let handler = trigger
                        .handlers
                        .get(used_handler)
                        .ok_or(Error::MissingHandler(trigger.name.clone(), used_handler.clone()))?;
                    handlers.push(ExtractedHandler {
                        id: trigger.name.clone(),
                        pattern: p.clone(),
                        handler: handler.clone(),
                    });
                }
            }
        }

        Ok(Self {
            handlers,
            triggers: trigger_set,
            hits: BTreeMap::new(),
        })
    }

    /// Process a batch set of paths and record the "hit"
    pub fn process_paths<T: AsRef<str>>(&mut self, paths: impl Iterator<Item = T>) {
        let results = paths.into_iter().flat_map(|p| {
            self.handlers.iter().filter_map(move |h| {
                h.pattern
                    .match_path(p.as_ref())
                    .map(|m| (h.id.clone(), h.handler.compiled(&m)))
            })
        });

        for (id, handler) in results {
            if let Some(map) = self.hits.get_mut(&id) {
                map.insert(handler);
            } else {
                self.hits.insert(id, BTreeSet::from_iter([handler]));
            }
        }
    }

    /// Bake the trigger collection into a sane dependency order
    pub fn bake(&mut self) -> Result<Vec<format::CompiledHandler>, Error> {
        let mut graph = dag::Dag::new();

        // ensure all keys are in place
        for id in self.hits.keys() {
            let _ = graph.add_node_or_get_index(id);
        }

        // add dependency ordering for the toplevel IDs
        for id in self.hits.keys() {
            let lookup = self
                .triggers
                .get(id)
                .ok_or(Error::MissingHandler(id.clone(), id.clone()))?;

            let node = graph.add_node_or_get_index(id);

            // This runs *before* B
            if let Some(before) = lookup
                .before
                .as_ref()
                .and_then(|b| self.triggers.get(b))
                .map(|f| graph.add_node_or_get_index(&f.name))
            {
                graph.add_edge(node, before);
            }

            // This runs *after* A
            if let Some(after) = lookup
                .after
                .as_ref()
                .and_then(|a| self.triggers.get(a))
                .map(|f| graph.add_node_or_get_index(&f.name))
            {
                graph.add_edge(after, node);
            }
        }

        // Recollect in dependency order
        let results = graph
            .topo()
            .filter_map(|i| self.hits.remove(i))
            .flatten()
            .collect::<Vec<_>>();
        Ok(results)
    }
}
