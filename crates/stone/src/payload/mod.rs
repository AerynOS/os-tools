// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

mod attribute;
mod index;
pub mod layout;
pub mod meta;

use std::io::{self, Read, Write};

use thiserror::Error;

pub use self::attribute::Attribute;
pub use self::index::Index;
pub use self::layout::Layout;
pub use self::meta::Meta;
use crate::{ReadExt, WriteExt};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    // The Metadata store
    Meta = 1,
    // File store, i.e. hash indexed
    Content = 2,
    // Map Files to Disk with basic UNIX permissions + types
    Layout = 3,
    // For indexing the deduplicated store
    Index = 4,
    // Attribute storage
    Attributes = 5,
    // For Writer interim
    Dumb = 6,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    // Payload has no compression
    None = 1,
    // Payload uses ZSTD compression
    Zstd = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub stored_size: u64,
    pub plain_size: u64,
    pub checksum: [u8; 8],
    pub num_records: usize,
    pub version: u16,
    pub kind: Kind,
    pub compression: Compression,
}

impl Header {
    /// Size of the encoded payload header in bytes
    pub const SIZE: usize = 8 + 8 + 8 + 4 + 2 + 1 + 1;

    pub fn decode<R: Read>(mut reader: R) -> Result<Self, DecodeError> {
        let stored_size = reader.read_u64()?;
        let plain_size = reader.read_u64()?;
        let checksum = reader.read_array()?;
        let num_records = reader.read_u32()? as usize;
        let version = reader.read_u16()?;

        let kind = match reader.read_u8()? {
            1 => Kind::Meta,
            2 => Kind::Content,
            3 => Kind::Layout,
            4 => Kind::Index,
            5 => Kind::Attributes,
            6 => Kind::Dumb,
            k => return Err(DecodeError::UnknownKind(k)),
        };

        let compression = match reader.read_u8()? {
            1 => Compression::None,
            2 => Compression::Zstd,
            d => return Err(DecodeError::UnknownCompression(d)),
        };

        Ok(Self {
            stored_size,
            plain_size,
            checksum,
            num_records,
            version,
            kind,
            compression,
        })
    }

    pub fn encode<W: Write>(&self, writer: &mut W) -> Result<(), EncodeError> {
        writer.write_u64(self.stored_size)?;
        writer.write_u64(self.plain_size)?;
        writer.write_array(self.checksum)?;
        writer.write_u32(self.num_records as u32)?;
        writer.write_u16(self.version)?;
        writer.write_u8(self.kind as u8)?;
        writer.write_u8(self.compression as u8)?;

        Ok(())
    }
}

pub trait Record: Sized {
    fn decode<R: Read>(reader: R) -> Result<Self, DecodeError>;
    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), EncodeError>;
    fn size(&self) -> usize;
}

pub fn decode_records<T: Record, R: Read>(mut reader: R, num_records: usize) -> Result<Vec<T>, DecodeError> {
    let mut records = Vec::with_capacity(num_records);

    for _ in 0..num_records {
        records.push(T::decode(&mut reader)?);
    }

    Ok(records)
}

pub fn encode_records<T: Record, W: Write>(writer: &mut W, records: &[T]) -> Result<(), EncodeError> {
    for record in records {
        record.encode(writer)?;
    }
    Ok(())
}

pub fn records_total_size<T: Record>(records: &[T]) -> usize {
    records.iter().map(T::size).sum()
}

#[derive(Debug, Clone)]
pub struct Payload<T> {
    pub header: Header,
    pub body: T,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Unknown header type: {0}")]
    UnknownKind(u8),
    #[error("Unknown header compression: {0}")]
    UnknownCompression(u8),
    #[error("Unknown metadata type: {0}")]
    UnknownMetaKind(u8),
    #[error("Unknown metadata tag: {0}")]
    UnknownMetaTag(u16),
    #[error("Unknown file type: {0}")]
    UnknownFileType(u8),
    #[error("Unknown dependency type: {0}")]
    UnknownDependency(u8),
    #[error("io")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("io")]
    Io(#[from] io::Error),
}
