use std::{collections::BTreeMap, mem};

use thiserror::Error;

use crate::{Build, KeyValue, Package, Recipe};

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
