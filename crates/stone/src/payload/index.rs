// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use super::{Record, StonePayloadDecodeError, StonePayloadEncodeError};
use crate::ext::{ReadExt, WriteExt};

/// An IndexEntry (a series of sequential entries within the IndexPayload)
/// record offsets to unique files within the ContentPayload when decompressed.
///
/// This is used to split the file into the content store on disk before promoting
/// to a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StonePayloadIndexRecord {
    /// Start pf the entry within the ContentPayload
    pub start: u64,

    /// End pointer, remove start for length
    pub end: u64,

    /// XXH3_128 hash
    pub digest: u128,
}

impl Record for StonePayloadIndexRecord {
    fn decode<R: Read>(mut reader: R) -> Result<Self, StonePayloadDecodeError> {
        let start = reader.read_u64()?;
        let end = reader.read_u64()?;
        let digest = reader.read_u128()?;

        Ok(Self { start, end, digest })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StonePayloadEncodeError> {
        writer.write_u64(self.start)?;
        writer.write_u64(self.end)?;
        writer.write_u128(self.digest)?;
        Ok(())
    }

    fn size(&self) -> usize {
        size_of::<Self>()
    }
}
