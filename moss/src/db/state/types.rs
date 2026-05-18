// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use rusqlite::Row;
use rusqlite::types::Type;
use serde::{self, Deserialize};
use serde_json;

use crate::db::Error;
use crate::state::{Kind, Selection, State};

impl<'a> TryFrom<&'a Row<'_>> for State {
    type Error = Error;

    fn try_from(row: &'a Row<'_>) -> Result<Self, Self::Error> {
        let selections_str = &row.get::<_, String>("selections")?;
        let selections: Vec<DbSelection> = serde_json::from_str(&selections_str)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?;

        Ok(Self {
            id: From::<i32>::from(row.get("id")?),
            summary: row.get("summary")?,
            description: row.get("description")?,
            selections: selections.into_iter().map(|s| s.into()).collect(),
            created: row.get("created")?,
            kind: decode_kind(row.get::<_, String>("type")?.as_str())?,
        })
    }
}

fn decode_kind(kind: &str) -> Result<Kind, Error> {
    Kind::try_from(kind).map_err(|_| Error::Decode("kind", kind.to_owned()))
}

#[derive(Deserialize)]
struct DbSelection {
    pub package_id: String,
    pub explicit: u8,
    pub reason: Option<String>,
}

impl From<DbSelection> for Selection {
    fn from(value: DbSelection) -> Self {
        Selection {
            package: value.package_id.into(),
            explicit: value.explicit != 0,
            reason: value.reason,
        }
    }
}
