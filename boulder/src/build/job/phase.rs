// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use itertools::Itertools;
use std::collections::BTreeSet;

use stone_recipe::macros::BuiltinDefinition as Def;
use stone_recipe::{
    Script, script,
    tuning::{self, Toolchain},
};
use tui::Styled;

use crate::build::pgo;
use crate::{Macros, Paths, Recipe, architecture::BuildTarget, util};

use super::{Error, work_dir};

pub fn list(pgo_stage: Option<pgo::Stage>) -> Vec<Phase> {
    if matches!(pgo_stage, Some(pgo::Stage::One | pgo::Stage::Two)) {
        Phase::WORKLOAD.to_vec()
    } else {
        Phase::NORMAL.to_vec()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum Phase {
    Prepare,
    Setup,
    Build,
    Install,
    Check,
    Workload,
}

impl Phase {
    const NORMAL: &'static [Self] = &[Phase::Prepare, Phase::Setup, Phase::Build, Phase::Install, Phase::Check];
    const WORKLOAD: &'static [Self] = &[Phase::Prepare, Phase::Setup, Phase::Build, Phase::Workload];

    pub fn abbrev(&self) -> &str {
        match self {
            Phase::Prepare => "P",
            Phase::Setup => "S",
            Phase::Build => "B",
            Phase::Install => "I",
            Phase::Check => "C",
            Phase::Workload => "W",
        }
    }

    pub fn styled(&self, s: impl ToString) -> String {
        let s = s.to_string();
        // Taste the rainbow
        // TODO: Ikey plz make pretty
        match self {
            Phase::Prepare => s.grey(),
            Phase::Setup => s.cyan(),
            Phase::Build => s.blue(),
            Phase::Check => s.yellow(),
            Phase::Install => s.green(),
            Phase::Workload => s.magenta(),
        }
        .dim()
        .to_string()
    }

    pub fn script(
        &self,
        target: BuildTarget,
        pgo_stage: Option<pgo::Stage>,
        recipe: &Recipe,
        paths: &Paths,
        macros: &Macros,
        ccache: bool,
    ) -> Result<Option<Script>, Error> {
        let root_build = &recipe.parsed.build;
        let target_build = recipe.build_target_definition(target);

        let Some(content) = (match self {
            Phase::Prepare => Some(prepare_script(&recipe.parsed.upstreams)),
            Phase::Setup => target_build.setup.clone().or_else(|| root_build.setup.clone()),
            Phase::Build => target_build.build.clone().or_else(|| root_build.build.clone()),
            Phase::Check => target_build.check.clone().or_else(|| root_build.check.clone()),
            Phase::Install => target_build.install.clone().or_else(|| root_build.install.clone()),
            Phase::Workload => match target_build.workload.clone().or_else(|| root_build.workload.clone()) {
                Some(mut content) => {
                    if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) {
                        if matches!(pgo_stage, Some(pgo::Stage::One)) {
                            content.push_str("%llvm_merge_s1");
                        } else if matches!(pgo_stage, Some(pgo::Stage::Two)) {
                            content.push_str("%llvm_merge_s2");
                        }
                    }

                    Some(content)
                }
                None => None,
            },
        }) else {
            return Ok(None);
        };

        if content.is_empty() {
            return Ok(None);
        }

        let mut env = target_build
            .environment
            .as_deref()
            .or(root_build.environment.as_deref())
            .filter(|env| *env != "(null)" && !env.is_empty() && !matches!(self, Phase::Prepare))
            .unwrap_or_default()
            .to_owned();
        env = format!("%scriptBase\n{env}\n");

        let mut parser = script::Parser::new().env(env);

        let build_target = target.to_string();
        let build_dir = paths.build().guest.join(&build_target);
        let work_dir = if matches!(self, Phase::Prepare) {
            build_dir.clone()
        } else {
            work_dir(&build_dir, &recipe.parsed.upstreams)
        };
        let num_jobs = util::num_cpus();

        for arch in ["base", &build_target] {
            let macros = macros
                .arch
                .get(arch)
                .cloned()
                .ok_or_else(|| Error::MissingArchMacros(arch.to_owned()))?;

            parser.add_macros(macros.clone());
        }

        for macros in macros.actions.clone() {
            parser.add_macros(macros.clone());
        }

        parser.add_definition(Def::Name, &recipe.parsed.source.name);
        parser.add_definition(Def::Version, &recipe.parsed.source.version);
        parser.add_definition(Def::Release, recipe.parsed.source.release);
        parser.add_definition(Def::Jobs, num_jobs);
        parser.add_definition(Def::PkgDir, paths.recipe().guest.join("pkg").display());
        parser.add_definition(Def::SourceDir, paths.upstreams().guest.display());
        parser.add_definition(Def::InstallRoot, paths.install().guest.display());
        parser.add_definition(Def::BuildRoot, build_dir.display());
        parser.add_definition(Def::WorkDir, work_dir.display());

        parser.add_definition(Def::CompilerCache, "/mason/ccache");
        parser.add_definition(Def::SCompilerCache, "/mason/sccache");

        parser.add_definition(Def::SourceDateEpoch, recipe.build_time.timestamp());

        let path = if ccache {
            "/usr/lib/ccache/bin:/usr/bin:/bin"
        } else {
            "/usr/bin:/bin"
        };

        if ccache {
            parser.add_definition(Def::RustcWrapper, "/usr/bin/sccache");
        } else {
            parser.add_definition(Def::RustcWrapper, "");
        }

        /* Set the relevant compilers */
        if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) {
            parser.add_definition(Def::CompilerC, "clang");
            parser.add_definition(Def::CompilerCxx, "clang++");
            parser.add_definition(Def::CompilerObjC, "clang");
            parser.add_definition(Def::CompilerObjCxx, "clang++");
            parser.add_definition(Def::CompilerCpp, "clang-cpp");
            parser.add_definition(Def::CompilerObjCpp, "clang -E -");
            parser.add_definition(Def::CompilerObjCxxCpp, "clang++ -E");
            parser.add_definition(Def::CompilerD, "ldc2");
            parser.add_definition(Def::CompilerAr, "llvm-ar");
            parser.add_definition(Def::CompilerObjcopy, "llvm-objcopy");
            parser.add_definition(Def::CompilerNm, "llvm-nm");
            parser.add_definition(Def::CompilerRanlib, "llvm-ranlib");
            parser.add_definition(Def::CompilerStrip, "llvm-strip");
        } else {
            parser.add_definition(Def::CompilerC, "gcc");
            parser.add_definition(Def::CompilerCxx, "g++");
            parser.add_definition(Def::CompilerObjC, "gcc");
            parser.add_definition(Def::CompilerObjCxx, "g++");
            parser.add_definition(Def::CompilerCpp, "gcc -E");
            parser.add_definition(Def::CompilerObjCpp, "gcc -E");
            parser.add_definition(Def::CompilerObjCxxCpp, "g++ -E");
            parser.add_definition(Def::CompilerD, "ldc2"); // FIXME: GDC
            parser.add_definition(Def::CompilerAr, "gcc-ar");
            parser.add_definition(Def::CompilerObjcopy, "objcopy");
            parser.add_definition(Def::CompilerNm, "gcc-nm");
            parser.add_definition(Def::CompilerRanlib, "gcc-ranlib");
            parser.add_definition(Def::CompilerStrip, "strip");
        }
        parser.add_definition(Def::CompilerPath, path);

        if recipe.parsed.mold {
            parser.add_definition(Def::CompilerLd, "ld.mold");
        } else if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) {
            parser.add_definition(Def::CompilerLd, "ld.lld");
        } else {
            parser.add_definition(Def::CompilerLd, "ld.bfd");
        }

        /* Allow packagers to do stage specific actions in a pgo build */
        if matches!(pgo_stage, Some(pgo::Stage::One)) {
            parser.add_definition(Def::PgoStage, "ONE");
        } else if matches!(pgo_stage, Some(pgo::Stage::Two)) {
            parser.add_definition(Def::PgoStage, "TWO");
        } else if matches!(pgo_stage, Some(pgo::Stage::Use)) {
            parser.add_definition(Def::PgoStage, "USE");
        } else {
            parser.add_definition(Def::PgoStage, "NONE");
        }

        parser.add_definition(Def::PgoDir, format!("{}-pgo", build_dir.display()));

        add_tuning(target, pgo_stage, recipe, macros, &mut parser)?;

        Ok(Some(parser.parse(&content)?))
    }
}

fn prepare_script(upstreams: &[stone_recipe::Upstream]) -> String {
    use std::fmt::Write;

    let mut content = String::default();

    for upstream in upstreams {
        match upstream {
            stone_recipe::Upstream::Plain {
                uri,
                rename,
                strip_dirs,
                unpack,
                unpack_dir,
                ..
            } => {
                if !*unpack {
                    continue;
                }
                let file_name = util::uri_file_name(uri);
                let rename = rename.as_deref().unwrap_or(file_name);
                let unpack_dir = unpack_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| rename.to_owned());
                let strip_dirs = strip_dirs.unwrap_or(1);

                let _ = writeln!(&mut content, "mkdir -p {unpack_dir}");
                let _ = writeln!(
                    &mut content,
                    r#"bsdtar-static xf "%(sourcedir)/{rename}" -C "{unpack_dir}" --strip-components={strip_dirs} --no-same-owner || (echo "Failed to extract archive"; exit 1);"#,
                );
            }
            stone_recipe::Upstream::Git { uri, clone_dir, .. } => {
                let source = util::uri_file_name(uri);
                let target = clone_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| source.to_owned());

                let _ = writeln!(&mut content, "mkdir -p {target}");
                let _ = writeln!(
                    &mut content,
                    r#"cp -Ra --no-preserve=ownership "%(sourcedir)/{source}/." "{target}""#,
                );
            }
        }
    }

    content
}

fn add_tuning(
    target: BuildTarget,
    pgo_stage: Option<pgo::Stage>,
    recipe: &Recipe,
    macros: &Macros,
    parser: &mut script::Parser,
) -> Result<(), Error> {
    let mut tuning = tuning::Builder::new();

    let build_target = target.to_string();

    for arch in ["base", &build_target] {
        let macros = macros
            .arch
            .get(arch)
            .cloned()
            .ok_or_else(|| Error::MissingArchMacros(arch.to_owned()))?;

        tuning.add_macros(macros);
    }

    for macros in macros.actions.clone() {
        tuning.add_macros(macros);
    }

    tuning.enable("architecture", None)?;

    for kv in &recipe.parsed.tuning {
        match &kv.value {
            stone_recipe::Tuning::Enable => tuning.enable(&kv.key, None)?,
            stone_recipe::Tuning::Disable => tuning.disable(&kv.key)?,
            stone_recipe::Tuning::Config(config) => tuning.enable(&kv.key, Some(config.clone()))?,
        }
    }

    // Add defaults that aren't already in recipe
    for group in default_tuning_groups(target, macros) {
        if !recipe.parsed.tuning.iter().any(|kv| &kv.key == group) {
            tuning.enable(group, None)?;
        }
    }

    if let Some(stage) = pgo_stage {
        match stage {
            pgo::Stage::One => tuning.enable("pgostage1", None)?,
            pgo::Stage::Two => tuning.enable("pgostage2", None)?,
            pgo::Stage::Use => {
                tuning.enable("pgouse", None)?;
                if recipe.parsed.options.samplepgo {
                    tuning.enable("pgosample", None)?;
                }
            }
        }
    }

    fn fmt_flags<'a>(flags: impl Iterator<Item = &'a str>) -> String {
        flags
            .map(|s| s.trim())
            .filter(|s| s.len() > 1)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .join(" ")
    }

    let toolchain = recipe.parsed.options.toolchain;
    let flags = tuning.build()?;

    let mut cflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::C, toolchain)),
    );
    let mut cxxflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::Cxx, toolchain)),
    );
    let fflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::F, toolchain)),
    );
    let ldflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::Ld, toolchain)),
    );
    let dflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::D, toolchain)),
    );
    let mut rustflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::Rust, toolchain)),
    );

    if recipe.parsed.mold {
        cflags.push_str(" -fuse-ld=mold");
        cxxflags.push_str(" -fuse-ld=mold");
        rustflags.push_str(" -Clink-arg=-fuse-ld=mold");
    }

    parser.add_definition(Def::CFlags, cflags);
    parser.add_definition(Def::CxxFlags, cxxflags);
    parser.add_definition(Def::FFlags, fflags);
    parser.add_definition(Def::LdFlags, ldflags);
    parser.add_definition(Def::DFlags, dflags);
    parser.add_definition(Def::RustFlags, rustflags);

    Ok(())
}

fn default_tuning_groups(target: BuildTarget, macros: &Macros) -> &[String] {
    let build_target = target.to_string();

    for arch in [&build_target, "base"] {
        let Some(arch_macros) = macros.arch.get(arch) else {
            continue;
        };

        if arch_macros.default_tuning_groups.is_empty() {
            continue;
        }

        return &arch_macros.default_tuning_groups;
    }

    &[]
}
