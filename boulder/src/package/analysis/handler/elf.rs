use std::{
    ffi::CStr,
    path::{Path, PathBuf},
    process::Command,
};

use elf::{
    abi::{DT_NEEDED, DT_RPATH, DT_RUNPATH, DT_SONAME},
    endian::AnyEndian,
    file::Class,
    note::Note,
    to_str,
};
use fs_err::File;

use moss::{Dependency, Provider, dependency};
use stone_recipe::tuning::Toolchain;

use crate::{
    package::{
        analysis::{BoxError, BucketMut, Decision, Response},
        collect::PathInfo,
    },
    util,
};

pub fn elf(bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    let file_name = info.file_name();

    if file_name.ends_with(".debug") && info.has_component("debug") {
        return Ok(Decision::NextHandler.into());
    }
    if !info.is_file() {
        return Ok(Decision::NextHandler.into());
    }

    let Ok(mut elf) = parse_elf(&info.path) else {
        return Ok(Decision::NextHandler.into());
    };

    let machine_isa = to_str::e_machine_to_str(elf.ehdr.e_machine)
        .map(|s| s.strip_prefix("EM_").unwrap_or(s))
        .unwrap_or_default()
        .to_lowercase();
    let bit_size = elf.ehdr.class;

    parse_dynamic_section(&mut elf, bucket, &machine_isa, bit_size, info, file_name);
    parse_interp_section(&mut elf, bucket, &machine_isa);

    let build_id = parse_build_id(&mut elf);

    let mut generated_paths = vec![];

    if let Some(build_id) = build_id {
        match split_debug(bucket, info, bit_size, &build_id) {
            Ok(Some(debug_path)) => {
                // Add new split file to be analyzed
                generated_paths.push(debug_path);
            }
            Ok(None) => {}
            // TODO: Error logging
            Err(err) => {
                eprintln!("error splitting debug info from {}: {err}", info.path.display());
            }
        }

        if let Err(err) = strip(bucket, info) {
            // TODO: Error logging
            eprintln!("error stripping {}: {err}", info.path.display());
        }

        // Restat original file after split & strip
        info.restat(bucket.hasher)?;
    }

    Ok(Response {
        decision: Decision::IncludeFile,
        generated_paths,
    })
}

fn parse_elf(path: &Path) -> Result<elf::ElfStream<AnyEndian, File>, BoxError> {
    let file = File::open(path)?;
    Ok(elf::ElfStream::open_stream(file)?)
}

fn parse_dynamic_section(
    elf: &mut elf::ElfStream<AnyEndian, File>,
    bucket: &mut BucketMut<'_>,
    machine_isa: &str,
    bit_size: Class,
    info: &PathInfo,
    file_name: &str,
) {
    let mut needed_offsets = vec![];
    let mut soname_offset = None;
    let mut rpath_offset = vec![];
    let mut runpath_offset = vec![];

    // i.e `/` `usr` `lib` `libfoo.so.1.2.3`
    let in_root_tree = info.target_path.ancestors().skip(1).count() == 3;

    // Get all dynamic entry offsets into string table
    if let Ok(Some(table)) = elf.dynamic() {
        for entry in table.iter() {
            match entry.d_tag {
                DT_NEEDED => {
                    needed_offsets.push(entry.d_val() as usize);
                }
                DT_SONAME => {
                    soname_offset = Some(entry.d_val() as usize);
                }
                DT_RPATH => {
                    rpath_offset.push(entry.d_val() as usize);
                }
                DT_RUNPATH => {
                    runpath_offset.push(entry.d_val() as usize);
                }
                _ => {}
            }
        }
    }

    // Resolve offsets against string table and add the applicable
    // depends and provides
    if let Ok(Some((_, strtab))) = elf.dynamic_symbol_table() {
        let origin = info.target_path.parent().unwrap().to_string_lossy().to_string();
        let mut rpaths = vec![origin.clone()];

        let root_dir = info
            .path
            .ancestors()
            .find(|p| p.ends_with("usr"))
            .and_then(|p| p.parent())
            .and_then(|p| p.to_str())
            .unwrap_or("/mason/install");

        for rpath in runpath_offset
            .iter()
            .chain(rpath_offset.iter())
            .filter_map(|v| strtab.get(*v).ok())
        {
            for path in rpath.split(':') {
                let path = path.replace("$ORIGIN", &origin);
                rpaths.push(path);
            }
        }

        // needed = dependency
        for offset in needed_offsets {
            if let Ok(name) = strtab.get(offset) {
                let rpath_name = rpaths.iter().find_map(|rpath| {
                    let local_p = root_dir.to_owned() + "/" + rpath + "/" + name;
                    let native_p = rpath.to_owned() + "/" + name;
                    let path = Path::new(&local_p);
                    let native_path = Path::new(&native_p);
                    if path.exists() {
                        Some(
                            Path::new("/")
                                .join(rpath)
                                .join(name)
                                .components()
                                .skip(3)
                                .collect::<PathBuf>(),
                        )
                    } else if native_path.exists() {
                        Some(Path::new(rpath).join(name).components().skip(3).collect::<PathBuf>())
                    } else {
                        None
                    }
                });

                let picked = if let Some(rpath_name) = rpath_name.as_ref() {
                    &rpath_name.to_string_lossy().to_string()
                } else {
                    name
                };

                bucket.dependencies.insert(Dependency {
                    kind: dependency::Kind::SharedLibrary,
                    name: format!("{picked}({machine_isa})"),
                });
            }
        }

        // soname exposed, let's share it
        if file_name.contains(".so") {
            let mut soname = "";

            if let Some(offset) = soname_offset {
                if let Ok(val) = strtab.get(offset) {
                    soname = val;
                }
            }

            if soname.is_empty() {
                soname = file_name;
            }

            // RPATH based soname
            if !in_root_tree {
                let rpathed = info
                    .target_path
                    .parent()
                    .unwrap()
                    .components()
                    .skip(3)
                    .collect::<PathBuf>()
                    .join(soname)
                    .to_string_lossy()
                    .to_string();
                bucket.providers.insert(Provider {
                    kind: dependency::Kind::SharedLibrary,
                    name: format!("{rpathed}({machine_isa})"),
                });
            } else {
                bucket.providers.insert(Provider {
                    kind: dependency::Kind::SharedLibrary,
                    name: format!("{soname}({machine_isa})"),
                });
            }

            // Do we possibly have an Interpreter? This is a .dynamic library ..
            if soname.starts_with("ld-") && info.target_path.to_str().unwrap_or_default().starts_with("/usr/lib") {
                let interp_paths = if matches!(bit_size, Class::ELF64) {
                    [
                        format!("/usr/lib64/{soname}({machine_isa})"),
                        format!("/lib64/{soname}({machine_isa})"),
                        format!("/lib/{soname}({machine_isa})"),
                        format!("{}({machine_isa})", info.target_path.display()),
                    ]
                } else {
                    [
                        format!("/usr/lib/{soname}({machine_isa})"),
                        format!("/lib32/{soname}({machine_isa})"),
                        format!("/lib/{soname}({machine_isa})"),
                        format!("{}({machine_isa})", info.target_path.display()),
                    ]
                };

                for path in interp_paths {
                    bucket.providers.insert(Provider {
                        kind: dependency::Kind::Interpreter,
                        name: path.clone(),
                    });
                    bucket.providers.insert(Provider {
                        kind: dependency::Kind::SharedLibrary,
                        name: path,
                    });
                }
            }
        }
    }
}

fn parse_interp_section(elf: &mut elf::ElfStream<AnyEndian, File>, bucket: &mut BucketMut<'_>, machine_isa: &str) {
    let Some(section) = elf.section_header_by_name(".interp").ok().flatten().copied() else {
        return;
    };

    let Ok((data, _)) = elf.section_data(&section) else {
        return;
    };

    if let Some(content) = CStr::from_bytes_until_nul(data).ok().and_then(|s| s.to_str().ok()) {
        bucket.dependencies.insert(Dependency {
            kind: dependency::Kind::Interpreter,
            name: format!("{content}({machine_isa})"),
        });
    }
}

fn parse_build_id(elf: &mut elf::ElfStream<AnyEndian, File>) -> Option<String> {
    let section = *elf.section_header_by_name(".note.gnu.build-id").ok()??;
    let notes = elf.section_data_as_notes(&section).ok()?;

    for note in notes {
        if let Note::GnuBuildId(build_id) = note {
            let build_id = hex::encode(build_id.0);
            return Some(build_id);
        }
    }

    None
}

fn split_debug(
    bucket: &BucketMut<'_>,
    info: &PathInfo,
    bit_size: Class,
    build_id: &str,
) -> Result<Option<PathBuf>, BoxError> {
    let use_llvm = matches!(bucket.recipe.parsed.options.toolchain, Toolchain::Llvm);
    let objcopy = if use_llvm {
        "/usr/bin/llvm-objcopy"
    } else {
        "/usr/bin/objcopy"
    };

    let debug_dir = if matches!(bit_size, Class::ELF64) {
        Path::new("usr/lib/debug/.build-id")
    } else {
        Path::new("usr/lib32/debug/.build-id")
    };
    let debug_info_relative_dir = debug_dir.join(&build_id[..2]);
    let debug_info_dir = bucket.paths.install().guest.join(debug_info_relative_dir);
    let debug_info_path = debug_info_dir.join(format!("{}.debug", &build_id[2..]));

    // Is it possible we already split this?
    if debug_info_path.exists() {
        return Ok(None);
    }

    util::ensure_dir_exists(&debug_info_dir)?;

    let output = Command::new(objcopy)
        .arg("--only-keep-debug")
        .arg(&info.path)
        .arg(&debug_info_path)
        .output()?;

    if !output.status.success() {
        return Err(String::from_utf8(output.stderr).unwrap_or_default().into());
    }

    let output = Command::new(objcopy)
        .arg("--add-gnu-debuglink")
        .arg(&debug_info_path)
        .arg(&info.path)
        .output()?;

    if !output.status.success() {
        return Err(String::from_utf8(output.stderr).unwrap_or_default().into());
    }

    Ok(Some(debug_info_path))
}

fn strip(bucket: &BucketMut<'_>, info: &PathInfo) -> Result<(), BoxError> {
    if !bucket.recipe.parsed.options.strip {
        return Ok(());
    }

    let use_llvm = matches!(bucket.recipe.parsed.options.toolchain, Toolchain::Llvm);
    let strip = if use_llvm {
        "/usr/bin/llvm-strip"
    } else {
        "/usr/bin/strip"
    };
    let is_executable = info
        .path
        .parent()
        .map(|parent| parent.ends_with("bin") || parent.ends_with("sbin"))
        .unwrap_or_default();

    let mut command = Command::new(strip);

    if is_executable {
        command.arg(&info.path);
    } else {
        command.args(["-g", "--strip-unneeded"]).arg(&info.path);
    }

    let output = command.output()?;

    if !output.status.success() {
        return Err(String::from_utf8(output.stderr).unwrap_or_default().into());
    }

    Ok(())
}
