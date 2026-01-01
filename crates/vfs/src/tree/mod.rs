// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Virtual filesystem tree (optimise layout inserts)

use core::fmt::Debug;
use std::collections::HashMap;
use std::vec;

use astr::AStr;
use elsa::FrozenVec;
use indextree::{Arena, Descendants, NodeId};
use snafu::Snafu;

use crate::path::{self, VfsPath};

pub mod builder;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Kind(usize);

impl Kind {
    /// Regular path
    pub const REGULAR: Self = Self(usize::MAX - 1);

    /// Directory (parenting node)
    pub const DIRECTORY: Self = Self(usize::MAX);

    // Every other value is a symlink
    pub fn symlink(target: AStr, symlink_targets: &FrozenVec<AStr>) -> Self {
        symlink_targets.push(target);
        Self(symlink_targets.len() - 1)
    }

    #[inline]
    pub fn is_symlink(self) -> bool {
        self.0 < usize::MAX - 1
    }

    pub fn as_symlink(self, symlink_targets: &FrozenVec<AStr>) -> Option<&str> {
        if self.is_symlink() {
            Some(&symlink_targets[self.0])
        } else {
            None
        }
    }
}

impl Default for Kind {
    fn default() -> Self {
        Self::DIRECTORY
    }
}

/// Simple generic interface for blittable files while retaining details.
///
/// All implementations should return a directory typed blitfile for a PathBuf.
pub trait BlitFile: Clone + Sized + Debug + From<AStr> {
    fn kind(&self, symlink_targets: &FrozenVec<AStr>) -> Kind;
    fn path(&self) -> AStr;
    fn id(&self) -> AStr;

    /// Clone the BlitFile and update the path
    fn cloned_to(&self, path: AStr) -> Self;
}

#[derive(Debug, Clone)]
struct File<T> {
    id: AStr,
    path: VfsPath,
    kind: Kind,
    inner: T,
}

impl<T: BlitFile> File<T> {
    pub fn new(inner: T, symlink_targets: &FrozenVec<AStr>) -> Self {
        let path = VfsPath::new(inner.path());

        Self {
            id: inner.id(),
            path,
            kind: inner.kind(symlink_targets),
            inner,
        }
    }

    #[inline]
    fn file_name(&self) -> &str {
        self.path.file_name()
    }

    #[inline]
    fn parent(&self) -> Option<&str> {
        self.path.parent()
    }
}

/// Actual tree implementation, encapsulating indextree
#[derive(Debug)]
pub struct Tree<T: BlitFile> {
    arena: Arena<File<T>>,
    map: HashMap<AStr, NodeId>,
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
        let path = data.path.astr();
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
                if n.file_name() == node.get().file_name() {
                    Some(n)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !others.is_empty() {
            let e = Error::Duplicate {
                node_path: node.get().path.astr(),
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
    fn reparent(
        &mut self,
        source_path: &str,
        target_path: &str,
        symlink_targets: &FrozenVec<AStr>,
    ) -> Result<(), Error> {
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
                orphans.push(File::new(original.inner.cloned_to(relapath), symlink_targets));
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
            if let Some(parent) = orphan.parent() {
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
        let partial = item.file_name();

        match item.kind {
            Kind::DIRECTORY => {
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
        node_path: AStr,
        node_id: AStr,
        other_id: AStr,
    },
}
