// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    path::{Path, PathBuf},
};

use moss::util;
use nix::NixPath;
use thiserror::Error;

pub struct Env {
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub moss_dir: PathBuf,
    pub config: config::Manager,
}

impl Env {
    pub fn new(
        cache_dir: Option<PathBuf>,
        config_dir: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        moss_root: Option<PathBuf>,
    ) -> Result<Self, Error> {
        let is_root = util::is_root();

        let config = if let Some(dir) = config_dir {
            config::Manager::custom(dir)
        } else if is_root {
            config::Manager::system("/", "boulder")
        } else {
            config::Manager::user("boulder")?
        };

        let cache_dir = resolve_cache_dir(is_root, cache_dir)?;
        let data_dir = resolve_data_dir(is_root, data_dir)?;
        let moss_dir = resolve_moss_root(is_root, moss_root)?;

        util::ensure_dir_exists(&cache_dir)?;
        util::ensure_dir_exists(&data_dir)?;
        util::ensure_dir_exists(&moss_dir)?;

        Ok(Self {
            config,
            cache_dir,
            data_dir,
            moss_dir,
        })
    }
}

fn resolve_cache_dir(is_root: bool, custom: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(dir) = custom {
        Ok(dir)
    } else if is_root {
        Ok(PathBuf::from("/var/cache/boulder"))
    } else {
        Ok(dirs::cache_dir().ok_or(Error::UserCache)?.join("boulder"))
    }
}

fn resolve_data_dir(is_root: bool, custom: Option<PathBuf>) -> Result<PathBuf, Error> {
    let root_dir = PathBuf::from("/usr/share/boulder");
    if let Some(dir) = custom {
        Ok(dir)
    } else if is_root {
        Ok(root_dir)
    } else {
        let user_datadir = dirs::data_dir().ok_or(Error::UserData)?.join("boulder");
        if user_datadir.exists() && !user_datadir.is_empty() {
            Ok(user_datadir)
        } else {
            Ok(root_dir)
        }
    }
}

fn resolve_moss_root(is_root: bool, custom: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(dir) = custom {
        if dir == Path::new("/.moss") {
            Err(Error::MossSystemRoot)
        } else {
            Ok(dir)
        }
    } else if is_root {
        Ok(PathBuf::from("/var/cache/boulder/moss"))
    } else {
        Ok(dirs::cache_dir().ok_or(Error::UserCache)?.join("moss"))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot find cache dir, $XDG_CACHE_HOME or $HOME env not set")]
    UserCache,
    #[error("cannot find config dir, $XDG_CONFIG_HOME or $HOME env not set")]
    UserConfig,
    #[error("cannot find data dir, $XDG_DATA_HOME or $HOME env not set")]
    UserData,
    #[error("boulder cannot use a moss system root")]
    MossSystemRoot,
    #[error("io")]
    Io(#[from] io::Error),
}

impl From<config::CreateUserError> for Error {
    fn from(_: config::CreateUserError) -> Self {
        Error::UserConfig
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn reject_moss_system_root() {
        assert!(matches!(
            resolve_moss_root(false, Some(PathBuf::from("/.moss"))),
            Err(Error::MossSystemRoot)
        ));
        assert!(matches!(
            resolve_moss_root(true, Some(PathBuf::from("/.moss"))),
            Err(Error::MossSystemRoot)
        ));
    }
}
