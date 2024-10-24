// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use astr::AStr;

use super::{Record, StonePayloadDecodeError, StonePayloadEncodeError};
use crate::ext::{ReadExt, WriteExt};

/// Layout entries record their target file type so they can be rebuilt on
/// the target installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
#[repr(u8)]
pub enum StonePayloadLayoutFileType {
    /// Regular file
    Regular = 1,

    /// Symbolic link (source + target set)
    Symlink = 2,

    /// Directory node
    Directory = 3,

    /// Character device
    CharacterDevice = 4,

    /// Block device
    BlockDevice = 5,

    /// FIFO node
    Fifo = 6,

    /// UNIX Socket
    Socket = 7,

    Unknown = 255,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StonePayloadLayoutFile {
    Regular(u128, AStr),
    Symlink(AStr, AStr),
    Directory(AStr),

    // not properly supported
    CharacterDevice(AStr),
    BlockDevice(AStr),
    Fifo(AStr),
    Socket(AStr),

    Unknown(AStr, AStr),
}

impl StonePayloadLayoutFile {
    fn source(&self) -> Vec<u8> {
        match self {
            StonePayloadLayoutFile::Regular(hash, _) => hash.to_be_bytes().to_vec(),
            StonePayloadLayoutFile::Symlink(source, _) => source.as_bytes().to_vec(),
            StonePayloadLayoutFile::Directory(_) => vec![],
            StonePayloadLayoutFile::CharacterDevice(_) => vec![],
            StonePayloadLayoutFile::BlockDevice(_) => vec![],
            StonePayloadLayoutFile::Fifo(_) => vec![],
            StonePayloadLayoutFile::Socket(_) => vec![],
            StonePayloadLayoutFile::Unknown(source, _) => source.as_bytes().to_vec(),
        }
    }

    pub fn target(&self) -> &str {
        match self {
            StonePayloadLayoutFile::Regular(_, target)
            | StonePayloadLayoutFile::Symlink(_, target)
            | StonePayloadLayoutFile::Directory(target)
            | StonePayloadLayoutFile::CharacterDevice(target)
            | StonePayloadLayoutFile::BlockDevice(target)
            | StonePayloadLayoutFile::Fifo(target)
            | StonePayloadLayoutFile::Socket(target)
            | StonePayloadLayoutFile::Unknown(_, target) => target,
        }
    }

    pub fn file_type(&self) -> StonePayloadLayoutFileType {
        match self {
            StonePayloadLayoutFile::Regular(..) => StonePayloadLayoutFileType::Regular,
            StonePayloadLayoutFile::Symlink(..) => StonePayloadLayoutFileType::Symlink,
            StonePayloadLayoutFile::Directory(_) => StonePayloadLayoutFileType::Directory,
            StonePayloadLayoutFile::CharacterDevice(_) => StonePayloadLayoutFileType::CharacterDevice,
            StonePayloadLayoutFile::BlockDevice(_) => StonePayloadLayoutFileType::BlockDevice,
            StonePayloadLayoutFile::Fifo(_) => StonePayloadLayoutFileType::Fifo,
            StonePayloadLayoutFile::Socket(_) => StonePayloadLayoutFileType::Socket,
            StonePayloadLayoutFile::Unknown(..) => StonePayloadLayoutFileType::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StonePayloadLayoutRecord {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub tag: u32,
    pub file: StonePayloadLayoutFile,
}

impl Record for StonePayloadLayoutRecord {
    fn decode<R: Read>(mut reader: R) -> Result<Self, StonePayloadDecodeError> {
        let uid = reader.read_u32()?;
        let gid = reader.read_u32()?;
        let mode = reader.read_u32()?;
        let tag = reader.read_u32()?;

        let source_length = reader.read_u16()?;
        let target_length = reader.read_u16()?;
        fn sanitize(s: &str) -> &str {
            s.trim_end_matches('\0')
        }

        let file_type = match reader.read_u8()? {
            1 => StonePayloadLayoutFileType::Regular,
            2 => StonePayloadLayoutFileType::Symlink,
            3 => StonePayloadLayoutFileType::Directory,
            4 => StonePayloadLayoutFileType::CharacterDevice,
            5 => StonePayloadLayoutFileType::BlockDevice,
            6 => StonePayloadLayoutFileType::Fifo,
            7 => StonePayloadLayoutFileType::Socket,
            _ => StonePayloadLayoutFileType::Unknown,
        };

        let _padding = reader.read_array::<11>()?;

        // Make the layout entry *usable*
        let entry = match file_type {
            StonePayloadLayoutFileType::Regular => {
                let source = reader.read_vec(source_length as usize)?;
                let hash = u128::from_be_bytes(source.try_into().unwrap());
                StonePayloadLayoutFile::Regular(hash, sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::Symlink => StonePayloadLayoutFile::Symlink(
                sanitize(&reader.read_string(source_length as u64)?).into(),
                sanitize(&reader.read_string(target_length as u64)?).into(),
            ),
            StonePayloadLayoutFileType::Directory => {
                StonePayloadLayoutFile::Directory(sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::CharacterDevice => {
                StonePayloadLayoutFile::CharacterDevice(sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::BlockDevice => {
                StonePayloadLayoutFile::BlockDevice(sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::Fifo => {
                StonePayloadLayoutFile::Fifo(sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::Socket => {
                StonePayloadLayoutFile::Socket(sanitize(&reader.read_string(target_length as u64)?).into())
            }
            StonePayloadLayoutFileType::Unknown => StonePayloadLayoutFile::Unknown(
                sanitize(&reader.read_string(source_length as u64)?).into(),
                sanitize(&reader.read_string(target_length as u64)?).into(),
            ),
        };

        Ok(Self {
            uid,
            gid,
            mode,
            tag,
            file: entry,
        })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StonePayloadEncodeError> {
        writer.write_u32(self.uid)?;
        writer.write_u32(self.gid)?;
        writer.write_u32(self.mode)?;
        writer.write_u32(self.tag)?;

        let source = self.file.source();
        let target = self.file.target();

        writer.write_u16(source.len() as u16)?;
        writer.write_u16(target.len() as u16)?;
        writer.write_u8(self.file.file_type() as u8)?;
        writer.write_array([0; 11])?;
        writer.write_all(&source)?;
        writer.write_all(target.as_bytes())?;

        Ok(())
    }

    fn size(&self) -> usize {
        4 + 4 + 4 + 4 + 2 + 2 + 1 + 11 + self.file.source().len() + self.file.target().len()
    }
}
