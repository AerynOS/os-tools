use std::collections::BTreeMap;

use crate::KeyValue;

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
