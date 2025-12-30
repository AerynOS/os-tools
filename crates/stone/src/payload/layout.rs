// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use astr::AStr;

use super::{DecodeError, EncodeError, Record};
use crate::{ReadExt, WriteExt};

/// Layout entries record their target file type so they can be rebuilt on
/// the target installation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Regular file
    Regular = 1,

    /// Symbolic link (source + target set)
    Symlink,

    /// Directory node
    Directory,

    /// Character device
    CharacterDevice,

    /// Block device
    BlockDevice,

    /// FIFO node
    Fifo,

    /// UNIX Socket
    Socket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Regular(u128, AStr),
    Symlink(AStr, AStr),
    Directory(AStr),

    // not properly supported
    CharacterDevice(AStr),
    BlockDevice(AStr),
    Fifo(AStr),
    Socket(AStr),
}

impl Entry {
    fn source(&self) -> Vec<u8> {
        match self {
            Entry::Regular(hash, _) => hash.to_be_bytes().to_vec(),
            Entry::Symlink(source, _) => source.as_bytes().to_vec(),
            Entry::Directory(_) => vec![],
            Entry::CharacterDevice(_) => vec![],
            Entry::BlockDevice(_) => vec![],
            Entry::Fifo(_) => vec![],
            Entry::Socket(_) => vec![],
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Entry::Regular(_, target) => target,
            Entry::Symlink(_, target) => target,
            Entry::Directory(target) => target,
            Entry::CharacterDevice(target) => target,
            Entry::BlockDevice(target) => target,
            Entry::Fifo(target) => target,
            Entry::Socket(target) => target,
        }
    }

    fn file_type(&self) -> u8 {
        match self {
            Entry::Regular(..) => 1,
            Entry::Symlink(..) => 2,
            Entry::Directory(_) => 3,
            Entry::CharacterDevice(_) => 4,
            Entry::BlockDevice(_) => 5,
            Entry::Fifo(_) => 6,
            Entry::Socket(_) => 7,
        }
    }
}

// TODO: Strong types these fields
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub tag: u32,
    pub entry: Entry,
}

impl Record for Layout {
    fn decode<R: Read>(mut reader: R) -> Result<Self, DecodeError> {
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
            1 => FileType::Regular,
            2 => FileType::Symlink,
            3 => FileType::Directory,
            4 => FileType::CharacterDevice,
            5 => FileType::BlockDevice,
            6 => FileType::Fifo,
            7 => FileType::Socket,
            t => return Err(DecodeError::UnknownFileType(t)),
        };

        let _padding = reader.read_array::<11>()?;

        // Make the layout entry *usable*
        let entry = match file_type {
            // BUG: boulder stores xxh128 as le bytes not be
            FileType::Regular => {
                let source = reader.read_vec(source_length as usize)?;
                let hash = u128::from_be_bytes(source.try_into().unwrap());
                Entry::Regular(hash, sanitize(&reader.read_string(target_length as u64)?).into())
            }
            FileType::Symlink => Entry::Symlink(
                sanitize(&reader.read_string(source_length as u64)?).into(),
                sanitize(&reader.read_string(target_length as u64)?).into(),
            ),
            FileType::Directory => Entry::Directory(sanitize(&reader.read_string(target_length as u64)?).into()),
            _ => {
                if source_length > 0 {
                    let _ = reader.read_vec(source_length as usize);
                }
                unreachable!()
            }
        };

        Ok(Self {
            uid,
            gid,
            mode,
            tag,
            entry,
        })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), EncodeError> {
        writer.write_u32(self.uid)?;
        writer.write_u32(self.gid)?;
        writer.write_u32(self.mode)?;
        writer.write_u32(self.tag)?;

        let source = self.entry.source();
        let target = self.entry.target();

        writer.write_u16(source.len() as u16)?;
        writer.write_u16(target.len() as u16)?;
        writer.write_u8(self.entry.file_type())?;
        writer.write_array([0; 11])?;
        writer.write_all(&source)?;
        writer.write_all(target.as_bytes())?;

        Ok(())
    }

    fn size(&self) -> usize {
        4 + 4 + 4 + 4 + 2 + 2 + 1 + 11 + self.entry.source().len() + self.entry.target().len()
    }
}
