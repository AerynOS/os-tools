// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Read, Write};

use thiserror::Error;

use crate::ext::{ReadExt, WriteExt};

pub use self::v1::{StoneHeaderV1, StoneHeaderV1DecodeError, StoneHeaderV1FileType};

pub mod v1;

/// Well defined magic field for a stone header
pub const STONE_HEADER_MAGIC: &[u8; 4] = b"\0mos";

/// Format versions are defined as u32, to allow further mangling in future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StoneHeaderVersion {
    V1 = 1,
}

/// The stone format uses an agnostic approach requiring a valid magic field
/// in the first 4 bytes, and a version specifier in the last 4 bytes, using
/// big endian order.
///
/// When the version is decoded, we can create the appropriate, version-specific
/// reader implementation, ensuring the container format is extensible well into
/// the future
#[repr(C)]
pub struct StoneAgnosticHeader {
    /// 4-bytes, BE (u32): Magic to quickly identify a stone file
    pub magic: [u8; 4],

    /// 24 bytes, version specific
    pub data: [u8; 24],

    /// 4-bytes, BE (u32): Format version used in the container
    pub version: [u8; 4],
}

impl StoneAgnosticHeader {
    fn decode<R: Read>(mut reader: R) -> io::Result<Self> {
        let magic = reader.read_array()?;
        let data = reader.read_array()?;
        let version = reader.read_array()?;

        Ok(Self { magic, data, version })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_array(self.magic)?;
        writer.write_array(self.data)?;
        writer.write_array(self.version)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoneHeader {
    V1(StoneHeaderV1),
}

impl StoneHeader {
    /// Size of the encoded header in bytes
    pub const SIZE: usize = size_of::<StoneAgnosticHeader>();

    pub fn version(&self) -> StoneHeaderVersion {
        match self {
            StoneHeader::V1(_) => StoneHeaderVersion::V1,
        }
    }

    pub fn num_payloads(&self) -> u16 {
        match self {
            StoneHeader::V1(header) => header.num_payloads,
        }
    }

    pub fn encode<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let version = u32::to_be_bytes(self.version() as u32);

        let data = match self {
            StoneHeader::V1(v1) => v1.encode(),
        };

        StoneAgnosticHeader {
            magic: *STONE_HEADER_MAGIC,
            data,
            version,
        }
        .encode(writer)
    }

    pub fn decode<R: Read>(reader: R) -> Result<Self, StoneHeaderDecodeError> {
        let header = StoneAgnosticHeader::decode(reader)?;

        if *STONE_HEADER_MAGIC != header.magic {
            return Err(StoneHeaderDecodeError::InvalidMagic);
        }

        let version = match u32::from_be_bytes(header.version) {
            1 => StoneHeaderVersion::V1,
            v => return Err(StoneHeaderDecodeError::UnknownVersion(v)),
        };

        Ok(match version {
            StoneHeaderVersion::V1 => Self::V1(StoneHeaderV1::decode(header.data)?),
        })
    }
}

#[derive(Debug, Error)]
pub enum StoneHeaderDecodeError {
    #[error("Header must be {} bytes long", size_of::<StoneAgnosticHeader>())]
    NotEnoughBytes,
    #[error("Invalid magic")]
    InvalidMagic,
    #[error("Unknown version: {0}")]
    UnknownVersion(u32),
    #[error("v1 decode")]
    V1(#[from] StoneHeaderV1DecodeError),
    #[error("io")]
    Io(io::Error),
}

impl From<io::Error> for StoneHeaderDecodeError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::UnexpectedEof => StoneHeaderDecodeError::NotEnoughBytes,
            _ => StoneHeaderDecodeError::Io(error),
        }
    }
}
