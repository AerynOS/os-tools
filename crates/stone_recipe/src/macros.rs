// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;

use crate::{
    Error, KeyValue, Package, sequence_of_key_value,
    tuning::{TuningFlag, TuningGroup},
};

pub fn from_slice(bytes: &[u8]) -> Result<Macros, Error> {
    serde_yaml::from_slice(bytes)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Macros {
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub actions: Vec<KeyValue<Action>>,
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub definitions: Vec<KeyValue<String>>,
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub flags: Vec<KeyValue<TuningFlag>>,
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub tuning: Vec<KeyValue<TuningGroup>>,
    #[serde(default, deserialize_with = "sequence_of_key_value")]
    pub packages: Vec<KeyValue<Package>>,
    #[serde(default)]
    pub default_tuning_groups: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Action {
    pub description: String,
    pub example: Option<String>,
    pub command: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// Macro to define the `BuiltinDefinition` enum and its methods.
///
/// Example input:
///
/// ```ignore
/// builtin_definitions! {
///     Name {
///         name: "name",
///         description: "Package name",
///         example: "I don't know how %(name) is used in practice",
///     },
///     Version {
///         name: "version",
///         description: "Package version",
///         example: "I don't know how %(version) is used in practice",
///     },
/// }
/// ```
///
/// Corresponding output:
///
/// ```no_run
/// # use stone_recipe::macros::DefinitionDetails;
/// pub enum BuiltinDefinition {
///     Name,
///     Version,
/// }
///
/// impl BuiltinDefinition {
///     pub fn name(&self) -> &str {
///         match self {
///             Self::Name => "name",
///             Self::Version => "version",
///         }
///     }
///
///     pub fn details(&self) -> DefinitionDetails<'_> {
///         match self {
///             Self::Name => DefinitionDetails {
///                 name: "name",
///                 description: "Package name",
///                 example: "I don't know how %(name) is used in practice",
///             },
///             Self::Version => DefinitionDetails {
///                 name: "version",
///                 description: "Package version",
///                 example: "I don't know how %(version) is used in practice",
///             },
///         }
///     }
/// }
/// ```
macro_rules! builtin_definitions {
    (
        $(
            $ident:ident {
                name: $name:expr,
                description: $description:expr,
                example: $example:expr$(,)?
            }
        ),+
        $(,)?
    ) => {
        pub enum BuiltinDefinition {
            $( $ident ),+
        }

        impl BuiltinDefinition {
            pub fn name(&self) -> &str {
                match self {
                    $( Self::$ident => $name, )+
                }
            }

            pub fn details(&self) -> DefinitionDetails<'_> {
                match self { $(
                    Self::$ident => DefinitionDetails {
                        name: $name,
                        description: $description,
                        example: $example,
                    },
                )+ }
            }
        }
    };
}

pub struct DefinitionDetails<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub example: &'a str,
}

builtin_definitions! {
    Name {
        name: "name",
        description: "The name of the main package built from the recipe.",
        example: "%(name)",
    },
    Version {
        name: "version",
        description: "The version of the package. Is considered string metadata.",
        example: "%(version)",
    },
    Release {
        name: "release",
        description: "The source release version of the recipe. Monotonically increasing integer used in place of version for comparisons.",
        example: "%(release)",
    },
    Jobs {
        name: "jobs",
        description: "The amount of parallel build jobs invoked. Defaults to $(nproc). Included by default in supported build action macros.",
        example: "make -j%(jobs)",
    },
    PkgDir {
        name: "pkgdir",
        description: "The ./pkg directory next to recipes used for holding package-specific additional files such as patches.",
        example: "%patch %(pkgdir)/some.patch",
    },
    SourceDir {
        name: "sourcedir",
        description: "The base directory into which each recipe upstream URI is unpacked as separate subdirectories.",
        example: "%(sourcedir)",
    },
    InstallRoot {
        name: "installroot",
        description: "The directory that is the root of the file tree that gets analysed for inclusion in .stones",
        example: "%install_file %(pkgdir)/a_file %(installroot)%(datadir)/%(name)/",
    },
    BuildRoot {
        name: "buildroot",
        description: "The directory that is the root of the file tree for the build process.",
        example: "%(buildroot)",
    },
    WorkDir {
        name: "workdir",
        description: "Each recipe phase starts in this directory and actions are implicitly run in this directory unless changed.",
        example: "%(workdir)",
    },
    CompilerCache {
        name: "compiler_cache",
        description: "Used when defining the %(ccachedir) location of ccache artefacts in the build container namespace.",
        example: "(only used when defining boulder build profiles)",
    },
    SCompilerCache {
        name: "scompiler_cache",
        description: "Used when defining the %(sccachedir) location of sccache artefacts in the build container namespace.",
        example: "(only used when defining boulder build profiles)",
    },
    SourceDateEpoch {
        name: "sourcedateepoch",
        description: "The canonical timestamp for when a recipe was built.",
        example: "Automatically defined and exported as ${SOURCE_DATE_EPOCH} by boulder.",
    },
    RustcWrapper {
        name: "rustc_wrapper",
        description: "Wrapper for the Rust compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerC {
        name: "compiler_c",
        description: "The C compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerCxx {
        name: "compiler_cxx",
        description: "The C++ compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerObjC {
        name: "compiler_objc",
        description: "The Objective C compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerObjCxx {
        name: "compiler_objcxx",
        description: "The Objective C++ compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerCpp {
        name: "compiler_cpp",
        description: "The C preprocessor used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerObjCpp {
        name: "compiler_objcpp",
        description: "The Objective C preprocessor used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerObjCxxCpp {
        name: "compiler_objcxxcpp",
        description: "The Objective C++ preprocessor used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerD {
        name: "compiler_d",
        description: "The DLang compiler used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerAr {
        name: "compiler_ar",
        description: "The ar ELF object archive tool used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerObjcopy {
        name: "compiler_objcopy",
        description: "The objcopy ELF object copy tool used by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerNm {
        name: "compiler_nm",
        description: "The nm tool used to decode (demangle) low-level symbol names into user-level names by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerRanlib {
        name: "compiler_ranlib",
        description: "The ranlib tool used to generate indices for statically linked object archives by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerStrip {
        name: "compiler_strip",
        description: "The strip tool used to remove debugging symbols from ELF objects by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerPath {
        name: "compiler_path",
        description: "The default ${PATH} when invoking compilers.",
        example: "(only used when defining boulder build profiles)",
    },
    CompilerLd {
        name: "compiler_ld",
        description: "The ld linker tool used to link ELF object files into executables or libraries by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    PgoStage {
        name: "pgo_stage",
        description: "Used to define the actions and flags in a PGO stage by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    PgoDir {
        name: "pgo_dir",
        description: "Used to define the dir used for the build and analysis of instrumented Profile Guided Optimisation runs by default in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CFlags {
        name: "cflags",
        description: "Defines the default C Compiler (CFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    CxxFlags {
        name: "cxxflags",
        description: "Defines the default C++ Compiler (CXXFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    FFlags {
        name: "fflags",
        description: "Defines the default FORTRAN Compiler (FFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    LdFlags {
        name: "ldflags",
        description: "Defines the default linker (LDFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    DFlags {
        name: "dflags",
        description: "Defines the default DLang Compiler (DFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
    RustFlags {
        name: "rustflags",
        description: "Defines the default Rust Compiler (RUSTFLAGS) environment variable flags in a build profile.",
        example: "(only used when defining boulder build profiles)",
    },
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn deserialize() {
        let inputs = [
            &include_bytes!("../../../test/base.yml")[..],
            &include_bytes!("../../../test/x86_64.yml")[..],
            &include_bytes!("../../../test/cmake.yml")[..],
        ];

        for input in inputs {
            let recipe = from_slice(input).unwrap();
            dbg!(&recipe);
        }
    }
}
