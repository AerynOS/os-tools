// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fmt, io,
    path::{Path, PathBuf},
};

use fs_err as fs;
use itertools::Itertools;
use serde_core::{Serialize, de::DeserializeOwned};
use snafu::{ResultExt, Snafu};

pub trait Config {
    fn domain() -> String;
}

#[derive(Debug, Clone)]
pub struct Manager {
    scope: Scope,
}

impl Manager {
    /// Config is loaded / merged from `usr/share` & `etc` relative to `root`
    /// and saved to `etc/{program}/{domain}.d/{name}.yaml
    pub fn system(root: impl Into<PathBuf>, program: impl ToString) -> Self {
        Self {
            scope: Scope::System {
                root: root.into(),
                program: program.to_string(),
            },
        }
    }

    /// Config is loaded from $XDG_CONFIG_HOME and saved to
    /// $XDG_CONFIG_HOME/{program}/{domain}.d/{name}.yaml
    pub fn user(program: impl ToString) -> Result<Self, CreateUserError> {
        Ok(Self {
            scope: Scope::User {
                config: dirs::config_dir().ok_or(CreateUserError)?,
                program: program.to_string(),
            },
        })
    }

    /// Config is loaded from `path` and saved to
    /// `path`/{domain}.d/{name}.yaml
    pub fn custom(path: impl Into<PathBuf>) -> Self {
        Self {
            scope: Scope::Custom(path.into()),
        }
    }

    pub fn load<T: Config + DeserializeOwned>(&self) -> Vec<LoadedConfig<T>> {
        let domain = T::domain();

        let mut configs = vec![];

        for (entry, resolve) in self.scope.load_with() {
            for item in enumerate_paths(entry, resolve, &domain) {
                if let Some(value) = read(item.format, &item.path) {
                    configs.push(LoadedConfig {
                        path: item.path,
                        format: item.format,
                        value,
                    });
                }
            }
        }

        // If both yaml & kdl configs are found, return
        // the KDL config since it is the newer format
        // & has the higher priority
        configs
            .into_iter()
            // Sort in priority ascending, so KDL entries
            // override Yaml entries
            .sorted_by_key(|config| config.format.priority())
            .fold(HashMap::new(), |mut acc, item| {
                let no_ext = item.path.with_extension("");
                acc.insert(no_ext, item);
                acc
            })
            .into_values()
            .collect()
    }

    pub fn save<T: Config + Serialize>(&self, name: impl fmt::Display, config: &T) -> Result<PathBuf, SaveError> {
        self.format_save(Format::Kdl, name, config)
    }

    fn format_save<T: Config + Serialize>(
        &self,
        format: Format,
        name: impl fmt::Display,
        config: &T,
    ) -> Result<PathBuf, SaveError> {
        let domain = T::domain();

        let dir = self.scope.save_dir(&domain);

        fs::create_dir_all(&dir).context(CreateDirSnafu { path: &dir })?;

        let path = dir.join(format!("{name}.{}", format.extension()));

        let serialized = match format {
            Format::Yaml => serde_yaml::to_string(config).context(YamlSnafu)?,
            Format::Kdl => {
                let mut doc = kdl::se::to_document(config).context(KdlSnafu)?;
                doc.autoformat();
                doc.to_string()
            }
        };

        fs::write(&path, serialized).context(WriteSnafu { path: path.clone() })?;

        Ok(path)
    }

    pub fn delete<T: Config>(&self, name: impl fmt::Display) -> io::Result<()> {
        let domain = T::domain();

        for format in Format::ALL {
            let dir = self.scope.save_dir(&domain);
            let path = dir.join(format!("{name}.{}", format.extension()));

            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Snafu)]
#[snafu(display("$HOME or $XDG_CONFIG_HOME env not set"))]
pub struct CreateUserError;

#[derive(Debug, Snafu)]
pub enum SaveError {
    #[snafu(display("create config dir"))]
    CreateDir { path: PathBuf, source: io::Error },
    #[snafu(display("serialize config to yaml"))]
    Yaml { source: serde_yaml::Error },
    #[snafu(display("serialize config to kdl"))]
    Kdl { source: kdl::se::Error },
    #[snafu(display("write config file"))]
    Write { path: PathBuf, source: io::Error },
}

pub struct LoadedConfig<T> {
    pub path: PathBuf,
    pub format: Format,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Kdl,
}

impl Format {
    pub const ALL: [Self; 2] = [Self::Yaml, Self::Kdl];

    pub const fn extension(&self) -> &'static str {
        match self {
            Format::Yaml => "yaml",
            Format::Kdl => "kdl",
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            Format::Yaml => 1,
            Format::Kdl => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnumeratedPath {
    pub path: PathBuf,
    pub format: Format,
}

fn enumerate_paths(entry: Entry, resolve: Resolve<'_>, domain: &str) -> Vec<EnumeratedPath> {
    match entry {
        Entry::File => {
            let mut paths = vec![];

            for format in Format::ALL {
                let path = resolve.file(domain, format.extension());

                if path.exists() {
                    paths.push(EnumeratedPath { path, format });
                }
            }

            paths
        }
        Entry::Directory => {
            if let Ok(read_dir) = fs::read_dir(resolve.dir(domain)) {
                read_dir
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        let exists = path.exists();
                        let extension = path.extension().and_then(|ext| ext.to_str())?;

                        if exists {
                            for format in Format::ALL {
                                if extension == format.extension() {
                                    return Some(EnumeratedPath { path, format });
                                }
                            }
                        }

                        None
                    })
                    .collect()
            } else {
                vec![]
            }
        }
    }
}

fn read<T: DeserializeOwned>(format: Format, path: &Path) -> Option<T> {
    match format {
        Format::Yaml => read_yaml(path),
        Format::Kdl => read_kdl(path),
    }
}

fn read_yaml<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_yaml::from_slice(&bytes).ok()
}

fn read_kdl<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let content = fs::read_to_string(path).ok()?;
    kdl::de::from_str(&content).ok()
}

#[derive(Debug, Clone)]

enum Scope {
    System { program: String, root: PathBuf },
    User { program: String, config: PathBuf },
    Custom(PathBuf),
}

impl Scope {
    fn save_dir<'a>(&'a self, domain: &'a str) -> PathBuf {
        match &self {
            Scope::System { root, program } => Resolve::System {
                root,
                base: SystemBase::Admin,
                program,
            },
            Scope::User { config, program } => Resolve::User { config, program },
            Scope::Custom(dir) => Resolve::Custom(dir),
        }
        .dir(domain)
    }

    fn load_with(&self) -> Vec<(Entry, Resolve<'_>)> {
        match &self {
            // System we search / merge all base file / .d files
            // from vendor then admin
            Scope::System { root, program } => vec![
                (
                    Entry::File,
                    Resolve::System {
                        root,
                        base: SystemBase::Vendor,
                        program,
                    },
                ),
                (
                    Entry::Directory,
                    Resolve::System {
                        root,
                        base: SystemBase::Vendor,
                        program,
                    },
                ),
                (
                    Entry::File,
                    Resolve::System {
                        root,
                        base: SystemBase::Admin,
                        program,
                    },
                ),
                (
                    Entry::Directory,
                    Resolve::System {
                        root,
                        base: SystemBase::Admin,
                        program,
                    },
                ),
            ],
            // System (root = "/") + User
            Scope::User { config, program } => {
                let root = Path::new("/");

                vec![
                    (
                        Entry::File,
                        Resolve::System {
                            root,
                            base: SystemBase::Vendor,
                            program,
                        },
                    ),
                    (
                        Entry::Directory,
                        Resolve::System {
                            root,
                            base: SystemBase::Vendor,
                            program,
                        },
                    ),
                    (
                        Entry::File,
                        Resolve::System {
                            root,
                            base: SystemBase::Admin,
                            program,
                        },
                    ),
                    (
                        Entry::Directory,
                        Resolve::System {
                            root,
                            base: SystemBase::Admin,
                            program,
                        },
                    ),
                    (Entry::File, Resolve::User { config, program }),
                    (Entry::Directory, Resolve::User { config, program }),
                ]
            }
            Scope::Custom(root) => vec![
                (Entry::File, Resolve::Custom(root)),
                (Entry::Directory, Resolve::Custom(root)),
            ],
        }
    }
}

#[derive(Clone, Copy)]
enum SystemBase {
    Admin,
    Vendor,
}

impl SystemBase {
    fn path(&self) -> &'static str {
        match self {
            SystemBase::Admin => "etc",
            SystemBase::Vendor => "usr/share",
        }
    }
}

enum Entry {
    File,
    Directory,
}

enum Resolve<'a> {
    System {
        root: &'a Path,
        base: SystemBase,
        program: &'a str,
    },
    User {
        config: &'a Path,
        program: &'a str,
    },
    Custom(&'a Path),
}

impl Resolve<'_> {
    fn config_dir(&self) -> PathBuf {
        match self {
            Resolve::System { root, base, program } => root.join(base.path()).join(program),
            Resolve::User { config, program } => config.join(program),
            Resolve::Custom(dir) => dir.to_path_buf(),
        }
    }

    fn file(&self, domain: &str, extension: &str) -> PathBuf {
        self.config_dir().join(format!("{domain}.{extension}"))
    }

    fn dir(&self, domain: &str) -> PathBuf {
        self.config_dir().join(format!("{domain}.d"))
    }
}
