// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Virtual filesystem tree (optimise layout inserts)

use core::fmt::Debug;
use std::collections::HashMap;
use std::vec;

use indextree::{Arena, Descendants, NodeId};
use snafu::Snafu;

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

/// Simple generic interface for blittable files while retaining details.
///
/// All implementations should return a directory typed blitfile for a PathBuf.
pub trait BlitFile: Clone + Sized + Debug + From<String> {
    fn kind(&self) -> Kind;
    fn path(&self) -> String;
    fn id(&self) -> String;

    /// Clone the BlitFile and update the path
    fn cloned_to(&self, path: String) -> Self;
}

#[derive(Debug, Clone)]
struct File<T> {
    // Cache these to avoid reallocation
    id: String,
    path: String,
    file_name: Option<String>,
    parent: Option<String>,
    kind: Kind,
    inner: T,
}

impl<T: BlitFile> File<T> {
    pub fn new(inner: T) -> Self {
        let path = inner.path();
        let file_name = path::file_name(&path).map(String::from);
        let parent = path::parent(&path).map(String::from);

        Self {
            id: inner.id(),
            path,
            file_name,
            parent,
            kind: inner.kind(),
            inner,
        }
    }
}

/// Actual tree implementation, encapsulating indextree
#[derive(Debug)]
pub struct Tree<T: BlitFile> {
    arena: Arena<File<T>>,
    map: HashMap<String, NodeId>,
    length: u64,
}

impl<T: BlitFile> Tree<T> {
    /// Construct a new Tree with specified capacity
    fn with_capacity(capacity: usize) -> Self {
        Tree {
            arena: Arena::with_capacity(capacity),
            map: HashMap::with_capacity(capacity),
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
    fn new_node(&mut self, data: File<T>) -> NodeId {
        let path = data.path.clone();
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
        let Some(parent_node) = self.map.get(parent) else {
            return Err(Error::MissingParent {
                parent: parent.to_owned(),
            });
        };

        let others = parent_node
            .children(&self.arena)
            .filter_map(|n| {
                let n = self.arena.get(n)?.get();
                if n.file_name == node.get().file_name {
                    Some(n)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !others.is_empty() {
            let e = Error::Duplicate {
                node_path: node.get().path.clone(),
                node_id: node.get().id.clone(),
                other_id: others.first().unwrap().id.clone(),
            };

            // TODO: Reenable
            // return Err(e)

            // Report duplicate and skip for now
            eprintln!("error: {e}");
        } else {
            parent_node.append(node_id, &mut self.arena);
        }

        Ok(())
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
                let relapath = path::join(target_path, original.path.strip_prefix(source_path).unwrap());
                orphans.push(File::new(original.inner.cloned_to(relapath)));
            }

            // Remove descendents
            let children = source.children(&self.arena).collect::<Vec<_>>();
            for child in children.iter() {
                child.remove_subtree(&mut self.arena);
            }
        }

        for orphan in orphans {
            let path = &orphan.path;
            // Do we have this node already?
            let node = match self.resolve_node(path) {
                Some(n) => *n,
                None => self.new_node(orphan.clone()),
            };
            if let Some(parent) = &orphan.parent {
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
    pub fn structured(&self) -> Option<Element<'_, T>> {
        self.resolve_node("/").map(|root| self.structured_children(root))
    }

    /// For the given node, recursively convert to Element::Directory of Child
    fn structured_children(&self, start: &NodeId) -> Element<'_, T> {
        let node = &self.arena[*start];
        let item = node.get();
        let partial = item.file_name.as_deref().unwrap_or_default();

        match item.kind {
            Kind::Directory => {
                let children = start
                    .children(&self.arena)
                    .map(|c| self.structured_children(&c))
                    .collect::<Vec<_>>();
                Element::Directory(partial, &item.inner, children)
            }
            _ => Element::Child(partial, &item.inner),
        }
    }
}

pub enum Element<'a, T: BlitFile> {
    Directory(&'a str, &'a T, Vec<Element<'a, T>>),
    Child(&'a str, &'a T),
}

/// Simple DFS iterator for a Tree
pub struct TreeIterator<'a, T: BlitFile> {
    parent: &'a Tree<T>,
    enume: Option<Descendants<'a, File<T>>>,
}

impl<'a, T: BlitFile> Iterator for TreeIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.enume {
            Some(enume) => enume
                .next()
                .and_then(|i| self.parent.arena.get(i))
                .map(|n| &n.get().inner),
            None => None,
        }
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("missing parent: {parent}"))]
    MissingParent { parent: String },

    #[snafu(display("duplicate entry: {node_path} {node_id} attempts to overwrite {other_id}"))]
    Duplicate {
        node_path: String,
        node_id: String,
        other_id: String,
    },
}
