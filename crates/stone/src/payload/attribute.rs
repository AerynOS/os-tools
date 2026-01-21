// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use super::{Record, StonePayloadDecodeError, StonePayloadEncodeError};
use crate::ext::{ReadExt, WriteExt};

#[derive(Debug, Clone)]
pub struct StonePayloadAttributeRecord {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl Record for StonePayloadAttributeRecord {
    fn decode<R: Read>(mut reader: R) -> Result<Self, StonePayloadDecodeError> {
        let key_length = reader.read_u64()?;
        let value_length = reader.read_u64()?;

        let key = reader.read_vec(key_length as usize)?;
        let value = reader.read_vec(value_length as usize)?;

        Ok(Self { key, value })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StonePayloadEncodeError> {
        writer.write_u64(self.key.len() as u64)?;
        writer.write_u64(self.value.len() as u64)?;
        writer.write_all(&self.key)?;
        writer.write_all(&self.value)?;

        Ok(())
    }

    fn size(&self) -> usize {
        8 + 8 + self.key.len() + self.value.len()
    }
}
