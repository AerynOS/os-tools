// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;

use crate::{
    sequence_of_key_value,
    tuning::{TuningFlag, TuningGroup},
    Error, KeyValue, Package,
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
        description: "",
        example: "",
    },
    Version {
        name: "version",
        description: "",
        example: "",
    },
    Release {
        name: "release",
        description: "",
        example: "",
    },
    Jobs {
        name: "jobs",
        description: "",
        example: "",
    },
    PkgDir {
        name: "pkgdir",
        description: "",
        example: "",
    },
    SourceDir {
        name: "sourcedir",
        description: "",
        example: "",
    },
    InstallRoot {
        name: "installroot",
        description: "",
        example: "",
    },
    BuildRoot {
        name: "buildroot",
        description: "",
        example: "",
    },
    WorkDir {
        name: "workdir",
        description: "",
        example: "",
    },
    CompilerCache {
        name: "compiler_cache",
        description: "",
        example: "",
    },
    SCompilerCache {
        name: "scompiler_cache",
        description: "",
        example: "",
    },
    SourceDateEpoch {
        name: "sourcedateepoch",
        description: "",
        example: "",
    },
    RustcWrapper {
        name: "rustc_wrapper",
        description: "",
        example: "",
    },
    CompilerC {
        name: "compiler_c",
        description: "",
        example: "",
    },
    CompilerCxx {
        name: "compiler_cxx",
        description: "",
        example: "",
    },
    CompilerObjC {
        name: "compiler_objc",
        description: "",
        example: "",
    },
    CompilerObjCxx {
        name: "compiler_objcxx",
        description: "",
        example: "",
    },
    CompilerCpp {
        name: "compiler_cpp",
        description: "",
        example: "",
    },
    CompilerObjCpp {
        name: "compiler_objcpp",
        description: "",
        example: "",
    },
    CompilerObjCxxCpp {
        name: "compiler_objcxxcpp",
        description: "",
        example: "",
    },
    CompilerD {
        name: "compiler_d",
        description: "",
        example: "",
    },
    CompilerAr {
        name: "compiler_ar",
        description: "",
        example: "",
    },
    CompilerObjcopy {
        name: "compiler_objcopy",
        description: "",
        example: "",
    },
    CompilerNm {
        name: "compiler_nm",
        description: "",
        example: "",
    },
    CompilerRanlib {
        name: "compiler_ranlib",
        description: "",
        example: "",
    },
    CompilerStrip {
        name: "compiler_strip",
        description: "",
        example: "",
    },
    CompilerPath {
        name: "compiler_path",
        description: "",
        example: "",
    },
    CompilerLd {
        name: "compiler_ld",
        description: "",
        example: "",
    },
    PgoStage {
        name: "pgo_stage",
        description: "",
        example: "",
    },
    PgoDir {
        name: "pgo_dir",
        description: "",
        example: "",
    },
    CFlags {
        name: "cflags",
        description: "",
        example: "",
    },
    CxxFlags {
        name: "cxxflags",
        description: "",
        example: "",
    },
    FFlags {
        name: "fflags",
        description: "",
        example: "",
    },
    LdFlags {
        name: "ldflags",
        description: "",
        example: "",
    },
    DFlags {
        name: "dflags",
        description: "",
        example: "",
    },
    RustFlags {
        name: "rustflags",
        description: "",
        example: "",
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
