use std::{collections::BTreeSet, io};

use fs_err as fs;
use thiserror::Error;

use crate::{Installation, dependency, repository};

use self::decode::decode;

mod decode;

#[derive(Debug, Clone)]
pub struct SystemModel {
    pub repositories: repository::Map,
    pub packages: BTreeSet<dependency::Provider>,
    pub raw: String,
}

pub fn load(installation: &Installation) -> Result<Option<SystemModel>, LoadError> {
    let path = installation.system_model_path();

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).map_err(LoadError::ReadFile)?;

    Ok(Some(decode(&content)?))
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("read file")]
    ReadFile(#[source] io::Error),
    #[error("decode")]
    Decode(#[from] decode::Error),
}
