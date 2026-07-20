// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use itertools::Itertools;
use std::collections::BTreeSet;
use stone_recipe::upstream;

use moss::util;
use stone_recipe::tuning::{self, Toolchain};
use tui::Styled;

use crate::build::pgo;
use crate::build::script::ScriptBundle;
use crate::{Macros, Paths, Recipe, architecture::BuildTarget};

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
    ) -> Result<Option<ScriptBundle>, Error> {
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

        let build_target = target.to_string();
        let build_dir = paths.build().guest.join(&build_target);
        let work_dir = if matches!(self, Phase::Prepare) {
            build_dir.clone()
        } else {
            work_dir(&build_dir, &recipe.parsed.upstreams)
        };
        let num_jobs = util::num_cpus();

        let mut env = macros.create_script_env(["base", &build_target])?;

        env.add_builtin_string("name", &recipe.parsed.source.name);
        env.add_builtin_string("version", &recipe.parsed.source.version);
        env.add_builtin_string("release", recipe.parsed.source.release);
        env.add_builtin_string("jobs", num_jobs);
        env.add_builtin_string("pkgdir", paths.recipe().guest.join("pkg").display());
        env.add_builtin_string("sourcedir", paths.upstreams().guest.display());
        env.add_builtin_string("installroot", paths.install().guest.display());
        env.add_builtin_string("buildroot", build_dir.display());
        env.add_builtin_string("workdir", work_dir.display());

        env.add_builtin_string("compiler_cache", "/mason/ccache");
        env.add_builtin_string("scompiler_cache", "/mason/sccache");

        env.add_builtin_string("sourcedateepoch", recipe.build_time.timestamp());

        let path = if ccache {
            "/usr/lib/ccache/bin:/usr/bin:/bin"
        } else {
            "/usr/bin:/bin"
        };

        if ccache {
            env.add_builtin_string("compiler_go_cache", "/mason/gocache");
            env.add_builtin_string("compiler_go_mod_cache", "/mason/gomodcache");
            env.add_builtin_string("compiler_cargo_cache", "/mason/cargocache");
            env.add_builtin_string("compiler_zig_cache", "/mason/zigcache");
            env.add_builtin_string("rustc_wrapper", "/usr/bin/sccache");
        } else {
            env.add_builtin_string("compiler_go_cache", "");
            env.add_builtin_string("compiler_go_mod_cache", "");
            env.add_builtin_string("compiler_cargo_cache", "");
            env.add_builtin_string("compiler_zig_cache", "");
            env.add_builtin_string("rustc_wrapper", "");
        }

        /* Set the relevant compilers */
        if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) {
            env.add_builtin_string("compiler_c", "clang");
            env.add_builtin_string("compiler_cxx", "clang++");
            env.add_builtin_string("compiler_objc", "clang");
            env.add_builtin_string("compiler_objcxx", "clang++");
            env.add_builtin_string("compiler_cpp", "clang-cpp");
            env.add_builtin_string("compiler_objcpp", "clang -E -");
            env.add_builtin_string("compiler_objcxxcpp", "clang++ -E");
            env.add_builtin_string("compiler_d", "ldc2");
            env.add_builtin_string("compiler_ar", "llvm-ar");
            env.add_builtin_string("compiler_objcopy", "llvm-objcopy");
            env.add_builtin_string("compiler_nm", "llvm-nm");
            env.add_builtin_string("compiler_ranlib", "llvm-ranlib");
            env.add_builtin_string("compiler_strip", "llvm-strip");
        } else {
            env.add_builtin_string("compiler_c", "gcc");
            env.add_builtin_string("compiler_cxx", "g++");
            env.add_builtin_string("compiler_objc", "gcc");
            env.add_builtin_string("compiler_objcxx", "g++");
            env.add_builtin_string("compiler_cpp", "gcc -E");
            env.add_builtin_string("compiler_objcpp", "gcc -E");
            env.add_builtin_string("compiler_objcxxcpp", "g++ -E");
            env.add_builtin_string("compiler_d", "ldc2"); // FIXME: GDC
            env.add_builtin_string("compiler_ar", "gcc-ar");
            env.add_builtin_string("compiler_objcopy", "objcopy");
            env.add_builtin_string("compiler_nm", "gcc-nm");
            env.add_builtin_string("compiler_ranlib", "gcc-ranlib");
            env.add_builtin_string("compiler_strip", "strip");
        }
        env.add_builtin_string("compiler_path", path);

        if recipe.parsed.mold {
            env.add_builtin_string("compiler_ld", "ld.mold");
        } else if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) {
            env.add_builtin_string("compiler_ld", "ld.lld");
        } else {
            env.add_builtin_string("compiler_ld", "ld.bfd");
        }

        /* Allow packagers to do stage specific actions in a pgo build */
        if matches!(pgo_stage, Some(pgo::Stage::One)) {
            env.add_builtin_string("pgo_stage", "ONE");
        } else if matches!(pgo_stage, Some(pgo::Stage::Two)) {
            env.add_builtin_string("pgo_stage", "TWO");
        } else if matches!(pgo_stage, Some(pgo::Stage::Use)) {
            env.add_builtin_string("pgo_stage", "USE");
        } else {
            env.add_builtin_string("pgo_stage", "NONE");
        }

        env.add_builtin_string("pgo_dir", format!("{}-pgo", build_dir.display()));

        add_tuning(target, pgo_stage, recipe, macros, &mut env)?;

        let mut prefix = target_build
            .environment
            .as_deref()
            .or(root_build.environment.as_deref())
            .filter(|env| *env != "(null)" && !env.is_empty() && !matches!(self, Phase::Prepare))
            .unwrap_or_default()
            .to_owned();
        prefix = format!("%scriptBase\n{prefix}\n");

        let prefix = stone_script::Expr::parse(&prefix)?;
        let expr = stone_script::Expr::parse(&content)?;

        Ok(Some(ScriptBundle::build(env, &prefix, &expr)?))
    }
}

fn prepare_script(upstreams: &[upstream::Upstream]) -> String {
    use std::fmt::Write;

    let mut content = String::default();

    for upstream in upstreams {
        match &upstream.props {
            upstream::Props::Plain {
                rename,
                strip_dirs,
                unpack,
                unpack_dir,
                ..
            } => {
                if !unpack {
                    continue;
                }
                let file_name = util::uri_file_name(&upstream.url);
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
            upstream::Props::Git { clone_dir, .. } => {
                let source = util::uri_file_name(&upstream.url);
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
    env: &mut stone_script::ScriptEnv,
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
    let valaflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::Vala, toolchain)),
    );
    let goflags = fmt_flags(
        flags
            .iter()
            .filter_map(|flag| flag.get(tuning::CompilerFlag::Go, toolchain)),
    );

    if recipe.parsed.mold {
        cflags.push_str(" -fuse-ld=mold");
        cxxflags.push_str(" -fuse-ld=mold");
        rustflags.push_str(" -Clink-arg=-fuse-ld=mold");
    }

    env.add_builtin("cflags", stone_script::Expr::parse(&cflags)?);
    env.add_builtin("cxxflags", stone_script::Expr::parse(&cxxflags)?);
    env.add_builtin("fflags", stone_script::Expr::parse(&fflags)?);
    env.add_builtin("ldflags", stone_script::Expr::parse(&ldflags)?);
    env.add_builtin("dflags", stone_script::Expr::parse(&dflags)?);
    env.add_builtin("rustflags", stone_script::Expr::parse(&rustflags)?);
    env.add_builtin("valaflags", stone_script::Expr::parse(&valaflags)?);
    env.add_builtin("goflags", stone_script::Expr::parse(&goflags)?);

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
