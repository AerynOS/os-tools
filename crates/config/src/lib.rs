// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use fs_err as fs;
use serde_core::{Serialize, de::DeserializeOwned};
use thiserror::Error;

const EXTENSION: &str = "yaml";

pub trait Config: DeserializeOwned {
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

    pub fn load<T: Config>(&self) -> Vec<T> {
        let domain = T::domain();

        let mut configs = vec![];

        for (entry, resolve) in self.scope.load_with() {
            for path in enumerate_paths(entry, resolve, &domain) {
                if let Some(config) = read_config(path) {
                    configs.push(config);
                }
            }
        }

        configs
    }

    pub fn save<T: Config + Serialize>(&self, name: impl fmt::Display, config: &T) -> Result<(), SaveError> {
        let domain = T::domain();

        let dir = self.scope.save_dir(&domain);

        fs::create_dir_all(&dir).map_err(|io| SaveError::CreateDir(dir.clone(), io))?;

        let path = dir.join(format!("{name}.{EXTENSION}"));

        let serialized = serde_yaml::to_string(config)?;

        fs::write(&path, serialized).map_err(|io| SaveError::Write(path, io))?;

        Ok(())
    }

    pub fn delete<T: Config>(&self, name: impl fmt::Display) -> io::Result<()> {
        let domain = T::domain();

        let dir = self.scope.save_dir(&domain);
        let path = dir.join(format!("{name}.{EXTENSION}"));

        fs::remove_file(path)?;

        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("$HOME or $XDG_CONFIG_HOME env not set")]
pub struct CreateUserError;

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("create config dir {0:?}")]
    CreateDir(PathBuf, #[source] io::Error),
    #[error("serialize config")]
    Yaml(#[from] serde_yaml::Error),
    #[error("write config file {0:?}")]
    Write(PathBuf, #[source] io::Error),
}

fn enumerate_paths(entry: Entry, resolve: Resolve<'_>, domain: &str) -> Vec<PathBuf> {
    match entry {
        Entry::File => {
            let file = resolve.file(domain);

            if file.exists() { vec![file] } else { vec![] }
        }
        Entry::Directory => {
            if let Ok(read_dir) = fs::read_dir(resolve.dir(domain)) {
                read_dir
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        let extension = path.extension().and_then(|ext| ext.to_str())?;

                        if path.exists() && extension == EXTENSION {
                            Some(path)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            }
        }
    }
}

fn read_config<T: Config>(path: PathBuf) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_yaml::from_slice(&bytes).ok()
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

    fn file(&self, domain: &str) -> PathBuf {
        self.config_dir().join(format!("{domain}.{EXTENSION}"))
    }

    fn dir(&self, domain: &str) -> PathBuf {
        self.config_dir().join(format!("{domain}.d"))
    }
}
