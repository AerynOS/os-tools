use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use fs_err as fs;
use moss::{dependency, Dependency, Provider};

use crate::package::collect::PathInfo;

use mailparse::{parse_mail, MailHeaderMap};

pub use self::elf::elf;
use super::{BoxError, BucketMut, Decision, Response};

mod elf;

pub fn include_any(_bucket: &mut BucketMut<'_>, _info: &mut PathInfo) -> Result<Response, BoxError> {
    Ok(Decision::IncludeFile.into())
}

pub fn ignore_blocked(_bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    // non-/usr = bad
    if !info.target_path.starts_with("/usr") {
        return Ok(Decision::IgnoreFile {
            reason: "non /usr/ file".into(),
        }
        .into());
    }

    // libtool files break the world
    if info.file_name().ends_with(".la")
        && (info.target_path.starts_with("/usr/lib") || info.target_path.starts_with("/usr/lib32"))
    {
        return Ok(Decision::IgnoreFile {
            reason: "libtool file".into(),
        }
        .into());
    }

    Ok(Decision::NextHandler.into())
}

pub fn binary(bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    if info.target_path.starts_with("/usr/bin") {
        let provider = Provider {
            kind: dependency::Kind::Binary,
            name: info.file_name().to_owned(),
        };
        bucket.providers.insert(provider);
    } else if info.target_path.starts_with("/usr/sbin") {
        let provider = Provider {
            kind: dependency::Kind::SystemBinary,
            name: info.file_name().to_owned(),
        };
        bucket.providers.insert(provider);
    }

    Ok(Decision::NextHandler.into())
}

pub fn pkg_config(bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    let file_name = info.file_name();

    if !info.has_component("pkgconfig") || !file_name.ends_with(".pc") {
        return Ok(Decision::NextHandler.into());
    }

    let provider_name = file_name.strip_suffix(".pc").expect("extension exists");
    let emul32 = info.has_component("lib32");

    let provider = Provider {
        kind: if emul32 {
            dependency::Kind::PkgConfig32
        } else {
            dependency::Kind::PkgConfig
        },
        name: provider_name.to_owned(),
    };

    bucket.providers.insert(provider);

    let output = Command::new("/usr/bin/pkg-config")
        .args(["--print-requires", "--print-requires-private", "--silence-errors"])
        .arg(&info.path)
        .envs([
            ("LC_ALL", "C"),
            (
                "PKG_CONFIG_PATH",
                if emul32 {
                    "/usr/lib32/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig"
                } else {
                    "/usr/lib/pkgconfig:/usr/share/pkgconfig"
                },
            ),
        ])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let deps = stdout.lines().filter_map(|line| line.split_whitespace().next());

    for dep in deps {
        let emul32_path = PathBuf::from(format!("/usr/lib32/pkgconfig/{dep}.pc"));
        let local_path = info
            .path
            .parent()
            .map(|p| p.join(format!("{dep}.pc")))
            .unwrap_or_default();

        let kind = if emul32 && (local_path.exists() || emul32_path.exists()) {
            dependency::Kind::PkgConfig32
        } else {
            dependency::Kind::PkgConfig
        };

        bucket.dependencies.insert(Dependency {
            kind,
            name: dep.to_owned(),
        });
    }

    Ok(Decision::NextHandler.into())
}

pub fn python(bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    let file_path = info.path.clone().into_os_string().into_string().unwrap_or_default();
    let is_dist_info = file_path.contains(".dist-info") && info.file_name().ends_with("METADATA");
    let is_egg_info = file_path.contains(".egg-info") && info.file_name().ends_with("PKG-INFO");

    if !(is_dist_info || is_egg_info) {
        return Ok(Decision::NextHandler.into());
    }

    let data = fs::read(&info.path)?;
    let mail = parse_mail(&data)?;
    let python_name = mail
        .get_headers()
        .get_first_value("Name")
        .unwrap_or_else(|| panic!("Failed to parse {}", info.file_name()))
        .to_lowercase();

    /* Insert generic provider */
    bucket.providers.insert(Provider {
        kind: dependency::Kind::Python,
        name: python_name.to_string(),
    });

    if python_name.contains("_") {
        /* Insert normalized generic provider */
        bucket.providers.insert(Provider {
            kind: dependency::Kind::Python,
            name: python_name.replace("_", "-"),
        });
    }

    let output = Command::new("/usr/bin/python3")
        .arg("-c")
        .arg("import platform; print(f'{platform.python_version_tuple()[0]}.{platform.python_version_tuple()[1]}')")
        .stdout(Stdio::piped())
        .output()?;
    let python_version = String::from_utf8_lossy(&output.stdout);

    /* Insert versioned provider for auto deps */
    bucket.providers.insert(Provider {
        kind: dependency::Kind::Python,
        name: format!("{python_name}({})", &python_version.to_string().trim_end()),
    });

    if python_name.contains("_") {
        /* Insert normalized versioned provider */
        bucket.providers.insert(Provider {
            kind: dependency::Kind::Python,
            name: format!(
                "{}({})",
                python_name.replace("_", "-"),
                &python_version.to_string().trim_end()
            ),
        });
    }

    /* Now parse dependencies */
    let dist_path = info
        .path
        .parent()
        .unwrap_or_else(|| panic!("Failed to get parent path for {}", info.file_name()));
    let find_deps_script = include_str!("scripts/get-py-deps.py");

    let output = Command::new("/usr/bin/python3")
        .arg("-c")
        .arg(find_deps_script)
        .arg(dist_path)
        .stdout(Stdio::piped())
        .output()?;

    let deps = String::from_utf8_lossy(&output.stdout);
    for dep in deps.lines() {
        bucket.dependencies.insert(Dependency {
            kind: dependency::Kind::Python,
            name: format!("{}({})", &dep.to_lowercase(), &python_version.to_string().trim_end()),
        });
    }

    Ok(Decision::NextHandler.into())
}

pub fn cmake(bucket: &mut BucketMut<'_>, info: &mut PathInfo) -> Result<Response, BoxError> {
    let file_name = info.file_name();

    if (!file_name.ends_with("Config.cmake") && !file_name.ends_with("-config.cmake"))
        || file_name.ends_with("-Config.cmake")
    {
        return Ok(Decision::NextHandler.into());
    }

    let provider_name = file_name
        .strip_suffix("Config.cmake")
        .or_else(|| file_name.strip_suffix("-config.cmake"))
        .expect("extension exists");

    bucket.providers.insert(Provider {
        kind: dependency::Kind::CMake,
        name: provider_name.to_owned(),
    });

    Ok(Decision::NextHandler.into())
}
