// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Virtual filesystem tree (optimise layout inserts)

use core::fmt::Debug;
use std::{collections::HashMap, vec};

use indextree::{Arena, Descendants, NodeId};
use thiserror::Error;

use crate::path;

pub mod builder;

#[derive(Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    // Regular path
    Regular,

    // Directory (parenting node)
    #[default]
    Directory,

    // Symlink to somewhere else.
    Symlink(String),
}

/// Simple generic interface for blittable files while retaining details
/// All implementations should return a directory typed blitfile for a PathBuf
pub trait BlitFile: Clone + Sized + Debug + From<String> {
    fn kind(&self) -> Kind;
    fn path(&self) -> String;
    fn id(&self) -> String;

    /// Clone the BlitFile and update the path
    fn cloned_to(&self, path: String) -> Self;
}

/// Actual tree implementation, encapsulating indextree
#[derive(Debug)]
pub struct Tree<T: BlitFile> {
    arena: Arena<T>,
    map: HashMap<String, NodeId>,
    length: u64,
}

impl<T: BlitFile> Tree<T> {
    /// Construct a new Tree
    fn new() -> Self {
        Tree {
            arena: Arena::new(),
            map: HashMap::new(),
            length: 0_u64,
        }
    }

    /// Return the number of items in the tree
    pub fn len(&self) -> u64 {
        self.length
    }

    /// Returns true if this tree is empty
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Generate a new node, store the path mapping for it
    fn new_node(&mut self, data: T) -> NodeId {
        let path = data.path();
        let node = self.arena.new_node(data);
        self.map.insert(path, node);
        self.length += 1;
        node
    }

    /// Resolve a node using the path
    fn resolve_node(&self, data: &str) -> Option<&NodeId> {
        self.map.get(data)
    }

    /// Add a child to the given parent node
    fn add_child_to_node(&mut self, node_id: NodeId, parent: &str) -> Result<(), Error> {
        let node = self.arena.get(node_id).unwrap();
        if let Some(parent_node) = self.map.get(parent) {
            let others = parent_node
                .children(&self.arena)
                .filter_map(|n| self.arena.get(n))
                .filter_map(|n| {
                    if path::file_name(&n.get().path()) == path::file_name(&node.get().path()) {
                        Some(n.get())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if !others.is_empty() {
                // TODO: Reenable
                // Err(Error::Duplicate(
                //     node.get().path(),
                //     node.get().id(),
                //     others.first().unwrap().id(),
                // ))

                // Report duplicate and skip for now
                eprintln!(
                    "error: {}",
                    Error::Duplicate(node.get().path(), node.get().id(), others.first().unwrap().id(),)
                );

                Ok(())
            } else {
                parent_node.append(node_id, &mut self.arena);
                Ok(())
            }
        } else {
            Err(Error::MissingParent(parent.to_string()))
        }
    }

    pub fn print(&self) {
        let root = self.resolve_node("/").unwrap();
        eprintln!("{:#?}", root.debug_pretty_print(&self.arena));
    }

    /// For all descendents of the given source tree, return a set of the reparented nodes,
    /// and remove the originals from the tree
    fn reparent(&mut self, source_path: &str, target_path: &str) -> Result<(), Error> {
        let mut mutations = vec![];
        let mut orphans = vec![];
        if let Some(source) = self.map.get(source_path) {
            if let Some(_target) = self.map.get(target_path) {
                for child in source.descendants(&self.arena).skip(1) {
                    mutations.push(child);
                }
            }

            for i in mutations {
                let original = self.arena.get(i).unwrap().get();
                let relapath = path::join(target_path, original.path().strip_prefix(source_path).unwrap());
                orphans.push(original.cloned_to(relapath));
            }

            // Remove descendents
            let children = source.children(&self.arena).collect::<Vec<_>>();
            for child in children.iter() {
                child.remove_subtree(&mut self.arena)
            }
        }

        for orphan in orphans {
            let path = orphan.path();
            // Do we have this node already?
            let node = match self.resolve_node(&path) {
                Some(n) => *n,
                None => self.new_node(orphan),
            };
            if let Some(parent) = path::parent(&path) {
                self.add_child_to_node(node, parent)?;
            }
        }

        Ok(())
    }

    /// Iterate using a TreeIterator, starting at the `/` node
    pub fn iter(&self) -> TreeIterator<'_, T> {
        TreeIterator {
            parent: self,
            enume: self.resolve_node("/").map(|n| n.descendants(&self.arena)),
        }
    }

    /// Return structured view beginning at `/`
    pub fn structured(&self) -> Option<Element<T>> {
        self.resolve_node("/").map(|root| self.structured_children(root))
    }

    /// For the given node, recursively convert to Element::Directory of Child
    fn structured_children(&self, start: &NodeId) -> Element<T> {
        let node = &self.arena[*start];
        let item = node.get();
        let partial = path::file_name(&item.path()).unwrap_or_default().to_string();

        match item.kind() {
            Kind::Directory => {
                let children = start
                    .children(&self.arena)
                    .map(|c| self.structured_children(&c))
                    .collect::<Vec<_>>();
                Element::Directory(partial, item.clone(), children)
            }
            _ => Element::Child(partial, item.clone()),
        }
    }
}

pub enum Element<T: BlitFile> {
    Directory(String, T, Vec<Element<T>>),
    Child(String, T),
}

/// Simple DFS iterator for a Tree
pub struct TreeIterator<'a, T: BlitFile> {
    parent: &'a Tree<T>,
    enume: Option<Descendants<'a, T>>,
}

impl<'a, T: BlitFile> Iterator for TreeIterator<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.enume {
            Some(enume) => enume
                .next()
                .and_then(|i| self.parent.arena.get(i))
                .map(|n| n.get())
                .cloned(),
            None => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing parent: {0}")]
    MissingParent(String),

    #[error("duplicate entry: {0} {1} attempts to overwrite {2}")]
    Duplicate(String, String, String),
}
