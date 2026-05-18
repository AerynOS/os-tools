// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::io;

use rusqlite::{Error, Row, types::Type};
use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};

pub struct Layout(StonePayloadLayoutRecord);

impl<'a> TryFrom<&'a Row<'_>> for Layout {
    type Error = Error;

    fn try_from(row: &'a Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self(StonePayloadLayoutRecord {
            uid: row.get::<_, i32>("uid")? as u32,
            gid: row.get::<_, i32>("gid")? as u32,
            mode: row.get::<_, i32>("mode")? as u32,
            tag: row.get::<_, i32>("tag")? as u32,
            file: TryInto::<File>::try_into(row)?.into_inner(),
        }))
    }
}

impl Layout {
    pub fn into_inner(self) -> StonePayloadLayoutRecord {
        self.0
    }
}

pub struct File(StonePayloadLayoutFile);

impl<'a> TryFrom<&'a Row<'_>> for File {
    type Error = Error;

    fn try_from(row: &'a Row<'_>) -> Result<Self, Self::Error> {
        let kind = row.get::<_, String>("entry_type")?;
        let src = row.get::<_, String>("entry_value1")?;
        let dest = match row.get::<_, Option<String>>("entry_value2")? {
            Some(dest) => dest,
            None if ["regular", "symlink", "unknown"].contains(&kind.as_str()) => {
                return Err(Error::InvalidColumnType(0, "entry_value2".to_owned(), Type::Null));
            }
            None => "".to_owned(),
        };

        let file = match kind.as_str() {
            "regular" => {
                let source = src
                    .parse()
                    .map_err(|e| Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?;
                StonePayloadLayoutFile::Regular(source, dest.into())
            }
            "symlink" => StonePayloadLayoutFile::Symlink(src.into(), dest.into()),
            "directory" => StonePayloadLayoutFile::Directory(src.into()),
            "character-device" => StonePayloadLayoutFile::CharacterDevice(src.into()),
            "block-device" => StonePayloadLayoutFile::BlockDevice(src.into()),
            "fifo" => StonePayloadLayoutFile::Fifo(src.into()),
            "socket" => StonePayloadLayoutFile::Socket(src.into()),
            "unknown" => StonePayloadLayoutFile::Unknown(src.into(), dest.into()),
            _ => {
                return Err(Error::FromSqlConversionFailure(
                    0,
                    Type::Text,
                    Box::new(io::Error::other(format!("{kind} is an invalid kind of file"))),
                ));
            }
        };
        Ok(Self(file))
    }
}

impl File {
    pub fn into_inner(self) -> StonePayloadLayoutFile {
        self.0
    }
}
