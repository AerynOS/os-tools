// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;

use crate::{
    Error, KeyValue, Package, sequence_of_key_value,
    tuning::{TuningFlag, TuningGroup},
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
