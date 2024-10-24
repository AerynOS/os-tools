// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StonePayloadIndexRecord {
    pub start: u64,
    pub end: u64,
    pub digest: [u8; 16],
}

impl From<&stone::StonePayloadIndexRecord> for StonePayloadIndexRecord {
    fn from(record: &stone::StonePayloadIndexRecord) -> Self {
        Self {
            start: record.start,
            end: record.end,
            digest: record.digest.to_be_bytes(),
        }
    }
}
