// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::{io, path::Path};

use fs_err as fs;
use thiserror::Error;

use crate::{Env, util};

#[derive(Debug)]
pub struct Macros {
    pub arch: BTreeMap<String, stone_recipe::Macros>,
    pub actions: Vec<stone_recipe::Macros>,
}

impl Macros {
    pub fn load(env: &Env) -> Result<Self, Error> {
        let macros_dir = env.data_dir.join("macros");
        let actions_dir = macros_dir.join("actions");
        let arch_dir = macros_dir.join("arch");

        let matcher = |p: &Path| p.extension().and_then(|s| s.to_str()) == Some("yaml");

        let arch_files = util::enumerate_files(&arch_dir, matcher).map_err(Error::ArchFiles)?;
        let action_files = util::enumerate_files(&actions_dir, matcher).map_err(Error::ActionFiles)?;

        let mut arch = BTreeMap::new();
        let mut actions = vec![];

        for file in arch_files {
            let relative = file.strip_prefix(&arch_dir).unwrap_or_else(|_| unreachable!());

            let identifier = relative.with_extension("").display().to_string();

            let bytes = fs::read(&file)?;
            let macros = stone_recipe::macros::from_slice(&bytes)?;

            arch.insert(identifier, macros);
        }

        for file in action_files {
            let bytes = fs::read(&file)?;
            let macros = stone_recipe::macros::from_slice(&bytes)?;

            actions.push(macros);
        }

        Ok(Self { arch, actions })
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("loading macros from arch data dir")]
    ArchFiles(#[source] io::Error),
    #[error("loading macros from actions data dir")]
    ActionFiles(#[source] io::Error),
    #[error("deserialize macros file")]
    Deserialize(#[from] stone_recipe::Error),
    #[error("io")]
    Io(#[from] io::Error),
}
