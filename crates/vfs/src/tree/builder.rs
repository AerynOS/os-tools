// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Build a vfs tree incrementally
use std::collections::BTreeMap;

use crate::path;
use crate::tree::{Kind, Tree};

use super::{BlitFile, Error};

/// Builder used to generate a full tree, free of conflicts
pub struct TreeBuilder<T: BlitFile> {
    // Explicitly requested incoming paths
    explicit: Vec<T>,

    // Implicitly created paths
    implicit_dirs: BTreeMap<String, T>,
}

/// Special sort algorithm for files by directory
fn sorted_paths<T: BlitFile>(a: &T, b: &T) -> std::cmp::Ordering {
    let a_path_len = a.path().matches('/').count();
    let b_path_len = b.path().matches('/').count();
    if a_path_len != b_path_len {
        a_path_len.cmp(&b_path_len)
    } else {
        a.path().cmp(&b.path())
    }
}

impl<T: BlitFile> Default for TreeBuilder<T> {
    fn default() -> Self {
        TreeBuilder::new()
    }
}

impl<T: BlitFile> TreeBuilder<T> {
    pub fn new() -> Self {
        TreeBuilder {
            explicit: vec![],
            implicit_dirs: BTreeMap::new(),
        }
    }

    /// Push an item to the builder - we don't care if we have duplicates yet
    pub fn push(&mut self, item: T) {
        let path = item.path();

        // Find all parent paths
        if let Some(parent) = path::parent(&path) {
            let mut leading_path: Option<String> = None;
            // Build a set of parent paths skipping `/`, yielding `usr`, `usr/bin`, etc.
            for component in path::components(parent) {
                let full_path = match leading_path {
                    Some(fp) => path::join(&fp, component),
                    None => component.to_string(),
                };
                leading_path = Some(full_path.clone());
                self.implicit_dirs.insert(full_path.clone(), full_path.into());
            }
        }
        self.explicit.push(item);
    }

    /// Sort incoming entries and remove duplicates
    pub fn bake(&mut self) {
        self.explicit.sort_by(sorted_paths);

        // Walk again to remove accidental dupes
        for i in self.explicit.iter() {
            self.implicit_dirs.remove(&i.path());
        }
    }

    /// Generate the final tree by baking all inputs
    pub fn tree(&self) -> Result<Tree<T>, Error> {
        // Chain all directories, replace implicits with explicit
        let all_dirs = self
            .explicit
            .iter()
            .filter(|f| matches!(f.kind(), Kind::Directory))
            .chain(self.implicit_dirs.values())
            .map(|d| (d.path(), d))
            .collect::<BTreeMap<_, _>>();

        // build a set of redirects
        let mut redirects = BTreeMap::new();

        // Resolve symlinks-to-dirs
        for link in self.explicit.iter() {
            if let Kind::Symlink(target) = link.kind() {
                let path = link.path();

                // Resolve the link.
                let target = if target.starts_with('/') {
                    target
                } else {
                    let parent = path::parent(&path);
                    if let Some(parent) = parent {
                        path::join(parent, &target)
                    } else {
                        target
                    }
                };
                if all_dirs.contains_key(&target) {
                    redirects.insert(path, target);
                }
            }
        }

        // Insert everything WITHOUT redirects, directory first.
        let mut full_set = all_dirs
            .into_values()
            .chain(self.explicit.iter().filter(|m| !matches!(m.kind(), Kind::Directory)))
            .collect::<Vec<_>>();
        full_set.sort_by(|a, b| sorted_paths(*a, *b));

        let mut tree: Tree<T> = Tree::new();

        // Build the initial full tree now.
        for entry in full_set {
            // New node for this guy
            let path = entry.path();
            let node = tree.new_node(entry.clone());

            if let Some(parent) = path::parent(&path) {
                tree.add_child_to_node(node, parent)?;
            }
        }

        // Reparent any symlink redirects.
        for (source_tree, target_tree) in redirects {
            tree.reparent(&source_tree, &target_tree)?;
        }
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use crate::tree::Kind;

    use super::{BlitFile, TreeBuilder};

    #[derive(Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct CustomFile {
        path: String,
        kind: Kind,
        id: String,
    }

    impl From<String> for CustomFile {
        fn from(value: String) -> Self {
            Self {
                path: value,
                kind: Kind::Directory,
                id: "Virtual".into(),
            }
        }
    }

    impl BlitFile for CustomFile {
        fn path(&self) -> String {
            self.path.clone()
        }

        fn kind(&self) -> Kind {
            self.kind.clone()
        }

        fn id(&self) -> String {
            self.id.clone()
        }

        /// Clone to new path portion
        fn cloned_to(&self, path: String) -> Self {
            Self {
                path,
                kind: self.kind.clone(),
                id: self.id.clone(),
            }
        }
    }

    #[test]
    fn test_simple_root() {
        let mut b: TreeBuilder<CustomFile> = TreeBuilder::new();
        let paths = vec![
            CustomFile {
                path: "/usr/bin/nano".into(),
                kind: Kind::Regular,
                id: "nano".into(),
            },
            CustomFile {
                path: "/usr/bin/rnano".into(),
                kind: Kind::Symlink("nano".to_string()),
                id: "nano".into(),
            },
            CustomFile {
                path: "/usr/share/nano".into(),
                kind: Kind::Directory,
                id: "nano".into(),
            },
            CustomFile {
                path: "/var/run/lock".into(),
                kind: Kind::Symlink("/run/lock".into()),
                id: "baselayout".into(),
            },
            CustomFile {
                path: "/var/run/lock/subsys/1".into(),
                kind: Kind::Regular,
                id: "baselayout".into(),
            },
        ];
        for path in paths {
            b.push(path);
        }
        b.bake();
        b.tree().unwrap();
    }
}
