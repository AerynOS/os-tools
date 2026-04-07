// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, mem};

use thiserror::Error;

use crate::{Build, KeyValue, Package, Recipe};

pub use self::decode::decode;

/// Control file to make modifications to a [`Recipe`]
#[derive(Debug, PartialEq, Eq, Default)]
pub struct ControlFile {
    pub modifications: BTreeMap<Modification, RecipeModification>,
}

/// Type of modification applied to a [`Recipe`] from a [`ControlFile`]
///
/// Note that [`Modification::Override`] will take precedence over
/// append & prepend modifications. If both are supplied, override
/// will "override" those other changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Modification {
    /// Prepends to recipe fields
    Prepend,
    /// Appends to recipe fields
    Append,
    /// Overrides recipe fields
    Override,
}

/// Recipe field modifications
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecipeModification {
    /// Modifications to global build fields
    pub build: BuildModification,
    /// Modifications to global package fields
    pub package: PackageModification,
    /// Modifications to per-profile build fields
    ///
    /// This applies [`Modification`] to the underlying profile,
    /// meaning it must exist in the source recipe. It will not
    /// add new profiles or override the entire collection.
    pub profiles: Vec<KeyValue<BuildModification>>,
    /// Modifications to per-subpackage fields
    ///
    /// This applies [`Modification`] to the underlying subpackage,
    /// meaning it must exist in the source recipe. It will not
    /// add new profiles or override the entire collection.
    pub sub_packages: Vec<KeyValue<PackageModification>>,
}

/// Build field modifications
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildModification {
    /// Modifies the setup phase string
    pub setup: Option<String>,
    /// Modifies the build phase string
    pub build: Option<String>,
    /// Modifies the install phase string
    pub install: Option<String>,
    /// Modifies the check phase string
    pub check: Option<String>,
    /// Modifies the workload phase string
    pub workload: Option<String>,
    /// Modifies the environment phase string
    pub environment: Option<String>,
    /// Modifies the builddeps array
    pub build_deps: Option<Vec<String>>,
    /// Modifies the checkdeps array
    pub check_deps: Option<Vec<String>>,
}

/// Package field modifications
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageModification {
    /// Modifies the rundeps array
    pub run_deps: Option<Vec<String>>,
    /// Modifies the rundeps-exclude array
    pub run_deps_exclude: Option<Vec<String>>,
    /// Modifies the conflicts array
    pub conflicts: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum ModificationError {
    #[error("profile {0} in {1} block does not exist in recipe")]
    MissingRecipeProfile(String, Modification),
    #[error("sub-package {0} in {1} block does not exist in recipe")]
    MissingRecipeSubPackage(String, Modification),
}

impl ControlFile {
    pub fn apply_to_recipe(self, recipe: &mut Recipe) -> Result<(), ModificationError> {
        self.modifications
            .into_iter()
            .try_for_each(|(kind, modification)| modification.apply_to_recipe(kind, recipe))
    }
}

impl RecipeModification {
    pub fn apply_to_recipe(self, modification: Modification, recipe: &mut Recipe) -> Result<(), ModificationError> {
        self.build.apply_to_build(modification, &mut recipe.build);
        self.package.apply_to_build(modification, &mut recipe.package);

        for kv in self.profiles {
            let Some(existing_profile) = recipe
                .profiles
                .iter_mut()
                .find_map(|b| (b.key == kv.key).then_some(&mut b.value))
            else {
                return Err(ModificationError::MissingRecipeProfile(kv.key, modification));
            };

            kv.value.apply_to_build(modification, existing_profile);
        }

        for kv in self.sub_packages {
            let Some(existing_package) = recipe
                .sub_packages
                .iter_mut()
                .find_map(|b| (b.key == kv.key).then_some(&mut b.value))
            else {
                return Err(ModificationError::MissingRecipeSubPackage(kv.key, modification));
            };

            kv.value.apply_to_build(modification, existing_package);
        }

        Ok(())
    }
}

impl BuildModification {
    pub fn apply_to_build(self, modification: Modification, build: &mut Build) {
        modification.update_string(&mut build.setup, self.setup);
        modification.update_string(&mut build.build, self.build);
        modification.update_string(&mut build.install, self.install);
        modification.update_string(&mut build.check, self.check);
        modification.update_string(&mut build.workload, self.workload);
        modification.update_string(&mut build.environment, self.environment);
        modification.update_string_array(&mut build.build_deps, self.build_deps);
        modification.update_string_array(&mut build.check_deps, self.check_deps);
    }
}

impl PackageModification {
    pub fn apply_to_build(self, modification: Modification, package: &mut Package) {
        modification.update_string_array(&mut package.run_deps, self.run_deps);
        modification.update_string_array(&mut package.run_deps_exclude, self.run_deps_exclude);
        modification.update_string_array(&mut package.conflicts, self.conflicts);
    }
}

impl Modification {
    pub fn update_string(self, source: &mut Option<String>, update: Option<String>) {
        if let Some(update) = update {
            let source_str = source.as_deref().unwrap_or_default();

            let new = match self {
                Modification::Prepend => format!("{update}\n{source_str}"),
                Modification::Append => format!("{source_str}\n{update}"),
                Modification::Override => update,
            };

            *source = Some(new);
        }
    }

    pub fn update_string_array(self, source: &mut Vec<String>, update: Option<Vec<String>>) {
        if let Some(mut update) = update {
            match self {
                Modification::Prepend => {
                    update.extend(mem::take(source));
                    *source = update;
                }
                Modification::Append => {
                    source.extend(update);
                }
                Modification::Override => {
                    *source = update;
                }
            }
        }
    }
}

pub mod decode {
    use kdl::{KdlDocument, KdlNode, KdlValue};
    use thiserror::Error;

    use crate::KeyValue;

    use super::{BuildModification, ControlFile, Modification, PackageModification, RecipeModification};

    #[derive(Debug, Error)]
    pub enum Error {
        #[error("parse as kdl document")]
        ParseKdlDocument(#[source] kdl::KdlError),
        #[error("invalid value for {0}.{1}, expected {2} got {3}")]
        InvalidNodeValue(Modification, &'static str, &'static str, String),
    }

    pub fn decode(content: &str) -> Result<ControlFile, Error> {
        let document: KdlDocument = content.parse().map_err(Error::ParseKdlDocument)?;

        let modifications = [Modification::Prepend, Modification::Append, Modification::Override]
            .into_iter()
            .flat_map(
                |modification| match decode_recipe_modification(modification, &document) {
                    Ok(Some(v)) => Some(Ok((modification, v))),
                    Ok(None) => None,
                    Err(err) => Some(Err(err)),
                },
            )
            .collect::<Result<_, Error>>()?;

        Ok(ControlFile { modifications })
    }

    fn decode_recipe_modification(
        modification: Modification,
        document: &KdlDocument,
    ) -> Result<Option<RecipeModification>, Error> {
        let key = modification.to_string();

        let Some(node) = document.get(&key) else {
            return Ok(None);
        };

        let build = decode_build_modification(modification, node)?;
        let package = decode_package_modification(node)?;

        let profiles = get_child_node(node, "profiles")
            .map(|node| {
                node.iter_children()
                    .map(|child| {
                        let key = child.name().to_string();
                        let value = decode_build_modification(modification, child)?;

                        Ok(KeyValue { key, value })
                    })
                    .collect()
            })
            .transpose()?
            .unwrap_or_default();

        let sub_packages = get_child_node(node, "packages")
            .map(|node| {
                node.iter_children()
                    .map(|child| {
                        let key = child.name().to_string();
                        let value = decode_package_modification(child)?;

                        Ok(KeyValue { key, value })
                    })
                    .collect()
            })
            .transpose()?
            .unwrap_or_default();

        Ok(Some(RecipeModification {
            build,
            package,
            profiles,
            sub_packages,
        }))
    }

    fn decode_build_modification(modification: Modification, node: &KdlNode) -> Result<BuildModification, Error> {
        let setup = get_string(modification, node, "setup")?;
        let build = get_string(modification, node, "build")?;
        let install = get_string(modification, node, "install")?;
        let check = get_string(modification, node, "check")?;
        let workload = get_string(modification, node, "workload")?;
        let environment = get_string(modification, node, "environment")?;
        let build_deps = get_string_array(node, "builddeps");
        let check_deps = get_string_array(node, "checkdeps");

        Ok(BuildModification {
            setup,
            build,
            install,
            check,
            workload,
            environment,
            build_deps,
            check_deps,
        })
    }

    fn decode_package_modification(node: &KdlNode) -> Result<PackageModification, Error> {
        let run_deps = get_string_array(node, "rundeps");
        let run_deps_exclude = get_string_array(node, "rundeps-exclude");
        let conflicts = get_string_array(node, "conflicts");

        Ok(PackageModification {
            run_deps,
            run_deps_exclude,
            conflicts,
        })
    }

    fn get_string(modification: Modification, node: &KdlNode, name: &'static str) -> Result<Option<String>, Error> {
        get_child_value(node, name)
            .map(|value| {
                value
                    .as_string()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| Error::InvalidNodeValue(modification, name, "string", value.to_string()))
            })
            .transpose()
    }

    fn get_string_array(node: &KdlNode, name: &'static str) -> Option<Vec<String>> {
        get_child_node(node, name).map(|node| {
            node.iter_children()
                .map(|child| child.name().value().to_owned())
                .collect()
        })
    }

    fn get_child_value<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlValue> {
        node.children()
            .and_then(|child| child.get(name))
            .and_then(|node| node.get(0))
    }

    fn get_child_node<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlNode> {
        node.children().and_then(|child| child.get(name))
    }

    #[cfg(test)]
    mod test {
        use std::collections::BTreeMap;

        use super::*;

        #[test]
        fn basic_parse() {
            let kdl = r#"
                append {
                  builddeps {
                      foo
                  }
                  rundeps {
                      bar
                  }
                  setup """
                    baz
                    thing
                    """
                }
                prepend {
                  profiles {
                      emul32 {
                          environment "test"
                      }
                  }
                }
                unknown
                override {
                    unknown
                    packages {
                        foo {
                            rundeps {
                                "binary(nano)"
                            }
                        }
                    }
                }
            "#;

            let control = decode(kdl).expect("valid kdl");

            assert_eq!(
                control,
                ControlFile {
                    modifications: BTreeMap::from_iter([
                        (
                            Modification::Append,
                            RecipeModification {
                                build: BuildModification {
                                    build_deps: Some(vec!["foo".to_owned()]),
                                    setup: Some("baz\nthing".to_owned()),
                                    ..Default::default()
                                },
                                package: PackageModification {
                                    run_deps: Some(vec!["bar".to_owned()]),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                        ),
                        (
                            Modification::Prepend,
                            RecipeModification {
                                profiles: vec![KeyValue {
                                    key: "emul32".to_owned(),
                                    value: BuildModification {
                                        environment: Some("test".to_owned()),
                                        ..Default::default()
                                    }
                                }],
                                ..Default::default()
                            }
                        ),
                        (
                            Modification::Override,
                            RecipeModification {
                                sub_packages: vec![KeyValue {
                                    key: "foo".to_owned(),
                                    value: PackageModification {
                                        run_deps: Some(vec!["binary(nano)".to_owned()]),
                                        ..Default::default()
                                    }
                                }],
                                ..Default::default()
                            }
                        )
                    ])
                }
            );
        }
    }
}
