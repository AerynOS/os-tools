// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::error;
use std::str::FromStr;

use rusqlite::{Error, Row, types::Type};

use crate::Dependency;
use crate::dependency::Kind;
use crate::package::{Meta, Name};

impl<'a> TryFrom<&'a Row<'_>> for Meta {
    type Error = Error;

    fn try_from(row: &'a Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            name: Name::from(row.get::<_, String>("name")?),
            version_identifier: row.get("version_identifier")?,
            source_release: row.get::<_, i64>("source_release")? as u64,
            build_release: row.get::<_, i64>("build_release")? as u64,
            architecture: row.get("architecture")?,
            summary: row.get("summary")?,
            description: row.get("description")?,
            source_id: row.get("source_id")?,
            homepage: row.get("homepage")?,
            licenses: json_array(&row.get("licenses")?)?,
            dependencies: parse_json_array(&row.get("dependencies")?)?.collect::<Result<_, _>>()?,
            providers: parse_json_array(&row.get("providers")?)?.collect::<Result<_, _>>()?,
            conflicts: parse_json_array(&row.get("conflicts")?)?.collect::<Result<_, _>>()?,
            uri: row.get("uri")?,
            hash: row.get("hash")?,
            download_size: row.get::<_, Option<i64>>("download_size")?.map(|size| size as u64),
        })
    }
}

impl<'a> TryFrom<&'a Row<'_>> for Dependency {
    type Error = Error;

    fn try_from(row: &'a Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: Kind::PackageName,
            name: row.get::<_, String>("package")?,
        })
    }
}

fn json_array(s: &String) -> Result<Vec<String>, Error> {
    let strings =
        serde_json::from_str(s).map_err(|err| Error::FromSqlConversionFailure(0, Type::Text, Box::new(err)))?;
    Ok(strings)
}

fn parse_json_array<T>(s: &String) -> Result<impl Iterator<Item = Result<T, Error>>, Error>
where
    T: FromStr + Sync,
    <T as FromStr>::Err: error::Error + Send + Sync + 'static,
{
    Ok(json_array(s)?.into_iter().map(|val| {
        (&val)
            .parse()
            .map_err(|err| Error::FromSqlConversionFailure(0, Type::Text, Box::new(err)))
    }))
}
