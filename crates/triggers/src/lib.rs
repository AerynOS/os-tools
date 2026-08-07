// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! System trigger management facilities

use std::collections::{BTreeMap, BTreeSet};

use format::Trigger;
use thiserror::Error;

pub mod format;

/// Grouped management of a set of triggers
pub struct Collection<'a> {
    handlers: Vec<ExtractedHandler<'a>>,
    triggers: BTreeMap<String, &'a Trigger>,
    hits: BTreeMap<String, BTreeMap<String, BTreeSet<fnmatch::Match>>>,
}

#[derive(Debug)]
struct ExtractedHandler<'a> {
    id: &'a str,
    handler_id: &'a str,
    pattern: &'a fnmatch::Pattern,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing handler reference in {0}: {1}")]
    MissingHandler(String, String),

    #[error("unknown argument variable mode {0} (expected 'each' or 'all')")]
    UnknownArgMode(String),
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
                    trigger
                        .handlers
                        .get(used_handler)
                        .ok_or(Error::MissingHandler(trigger.name.clone(), used_handler.clone()))?;
                    handlers.push(ExtractedHandler {
                        id: &trigger.name,
                        handler_id: used_handler,
                        pattern: p,
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
    pub fn process_paths(&mut self, paths: impl Iterator<Item = String>) {
        for p in paths {
            for h in &self.handlers {
                if let Some(m) = h.pattern.match_path(&p) {
                    self.hits
                        .entry(h.id.to_owned())
                        .or_default()
                        .entry(h.handler_id.to_owned())
                        .or_default()
                        .insert(m);
                }
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

        let mut results = Vec::new();
        for id in graph.topo() {
            let Some(trigger) = self.triggers.get(id) else {
                continue;
            };
            let Some(hits) = self.hits.get(id) else {
                continue;
            };

            let mut stage_handlers = Vec::new();
            for (handler_id, matches) in hits {
                let Some(handler) = trigger.handlers.get(handler_id) else {
                    continue;
                };
                stage_handlers.extend(handler.coalesced(matches)?);
            }
            results.extend(stage_handlers);
        }
        Ok(results)
    }
}
