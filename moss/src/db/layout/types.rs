// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{borrow::Cow, io};

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
            file: TryInto::<DecodedFile>::try_into(row)?.into_inner(),
        }))
    }
}

impl Layout {
    pub fn into_inner(self) -> StonePayloadLayoutRecord {
        self.0
    }
}

pub struct EncodedFile<'a> {
    pub type_: &'static str,
    pub value1: Option<Cow<'a, str>>,
    pub value2: Option<&'a str>,
}

pub fn encode_file<'a>(entry: &'a StonePayloadLayoutFile) -> EncodedFile<'a> {
    match entry {
        StonePayloadLayoutFile::Regular(hash, name) => EncodedFile {
            type_: "regular",
            value1: Some(hash.to_string().into()),
            value2: Some(name),
        },
        StonePayloadLayoutFile::Symlink(a, b) => EncodedFile {
            type_: "symlink",
            value1: Some(a.into()),
            value2: Some(b),
        },
        StonePayloadLayoutFile::Directory(name) => EncodedFile {
            type_: "directory",
            value1: Some(name.into()),
            value2: None,
        },
        StonePayloadLayoutFile::CharacterDevice(name) => EncodedFile {
            type_: "character-device",
            value1: Some(name.into()),
            value2: None,
        },
        StonePayloadLayoutFile::BlockDevice(name) => EncodedFile {
            type_: "block-device",
            value1: Some(name.into()),
            value2: None,
        },
        StonePayloadLayoutFile::Fifo(name) => EncodedFile {
            type_: "fifo",
            value1: Some(name.into()),
            value2: None,
        },
        StonePayloadLayoutFile::Socket(name) => EncodedFile {
            type_: "socket",
            value1: Some(name.into()),
            value2: None,
        },
        StonePayloadLayoutFile::Unknown(a, b) => EncodedFile {
            type_: "unknown",
            value1: Some(a.into()),
            value2: Some(b),
        },
    }
}

struct DecodedFile(StonePayloadLayoutFile);

impl<'a> TryFrom<&'a Row<'_>> for DecodedFile {
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

impl DecodedFile {
    pub fn into_inner(self) -> StonePayloadLayoutFile {
        self.0
    }
}
