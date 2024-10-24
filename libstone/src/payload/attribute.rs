// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StonePayloadAttributeRecord {
    pub key_size: usize,
    pub key_buf: *const u8,
    pub value_size: usize,
    pub value_buf: *const u8,
}

impl From<&stone::StonePayloadAttributeRecord> for StonePayloadAttributeRecord {
    fn from(record: &stone::StonePayloadAttributeRecord) -> Self {
        Self {
            key_size: record.key.len(),
            key_buf: record.key.as_ptr(),
            value_size: record.value.len(),
            value_buf: record.value.as_ptr(),
        }
    }
}
