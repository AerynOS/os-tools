// SPDX-FileCopyrightText: Copyright © 2020-2023 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Virtual filesystem tree (optimise layout inserts)

use core::fmt::Debug;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use indextree::{Arena, Descendants, NodeId};
use thiserror::Error;

pub use builder::TreeBuilder;

mod builder;

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
pub trait BlitFile: Clone + Sized + Debug + From<PathBuf> {
    fn kind(&self) -> Kind;
    fn path(&self) -> &Path;

    /// Clone the BlitFile and update the path
    fn cloned_to(&self, path: PathBuf) -> Self;
}

/// Actual tree implementation, encapsulating indextree
#[derive(Debug)]
pub struct Tree<T: BlitFile> {
    arena: Arena<T>,
    map: HashMap<PathBuf, NodeId>,
}

impl<T: BlitFile> Tree<T> {
    /// Construct a new Tree
    fn new() -> Self {
        Tree {
            arena: Arena::new(),
            map: HashMap::new(),
        }
    }

    /// Generate a new node, store the path mapping for it
    fn new_node(&mut self, data: T) -> NodeId {
        let path = data.path().to_path_buf();
        let node = self.arena.new_node(data);
        self.map.insert(path.to_path_buf(), node);
        node
    }

    /// Resolve a node using the path
    fn resolve_node(&self, data: impl Into<PathBuf>) -> Option<&NodeId> {
        self.map.get(&data.into())
    }

    /// Add a child to the given parent node
    fn add_child_to_node(
        &mut self,
        node_id: NodeId,
        parent: impl Into<PathBuf>,
    ) -> Result<(), Error> {
        let parent = parent.into();
        let node = self.arena.get(node_id).unwrap();
        let path = node.get().path();

        if let Some(parent_node) = self.map.get(&parent) {
            let is_duplicate = parent_node
                .children(&self.arena)
                .filter_map(|n| self.arena.get(n))
                .any(|child| child.get().path().file_name() == path.file_name());

            if is_duplicate {
                Err(Error::Duplicate(path.to_path_buf()))
            } else {
                parent_node.append(node_id, &mut self.arena);
                Ok(())
            }
        } else {
            Err(Error::MissingParent(parent))
        }
    }

    pub fn print(&self) {
        let root = self.resolve_node("/").unwrap();
        eprintln!("{:#?}", root.debug_pretty_print(&self.arena));
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
        self.resolve_node("/")
            .map(|root| self.structured_children(root))
    }

    /// For the given node, recursively convert to Element::Directory of Child
    fn structured_children(&self, start: &NodeId) -> Element<T> {
        let node = &self.arena[*start];
        let item = node.get();
        let partial = item
            .path()
            .file_name()
            .unwrap_or(OsStr::new(""))
            .to_string_lossy()
            .to_string();
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
    MissingParent(PathBuf),

    #[error("duplicate entry")]
    Duplicate(PathBuf),
}
