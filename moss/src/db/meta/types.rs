// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use rusqlite::{Error, Row};

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
            licenses: Default::default(),
            dependencies: Default::default(),
            providers: Default::default(),
            conflicts: Default::default(),
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
