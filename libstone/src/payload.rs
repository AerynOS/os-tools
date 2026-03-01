// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use stone::{StoneDecodedPayload, StonePayloadHeader};

pub use self::attribute::StonePayloadAttributeRecord;
pub use self::index::StonePayloadIndexRecord;
pub use self::layout::StonePayloadLayoutRecord;
pub use self::meta::{StonePayloadMetaPrimitiveType, StonePayloadMetaRecord};

mod attribute;
mod index;
mod layout;
mod meta;

pub struct StonePayload {
    pub decoded: StoneDecodedPayload,
    next_record: usize,
}

impl StonePayload {
    pub fn header(&self) -> &StonePayloadHeader {
        self.decoded.header()
    }

    pub fn next_layout_record(&mut self) -> Option<StonePayloadLayoutRecord> {
        if self.next_record >= self.header().num_records {
            return None;
        }

        let payload = self.decoded.layout()?;
        let record = payload.body.get(self.next_record)?;

        self.next_record += 1;

        Some(record.into())
    }

    pub fn next_meta_record(&mut self) -> Option<StonePayloadMetaRecord> {
        if self.next_record >= self.header().num_records {
            return None;
        }

        let payload = self.decoded.meta()?;
        let record = payload.body.get(self.next_record)?;

        self.next_record += 1;

        Some(record.into())
    }

    pub fn next_index_record(&mut self) -> Option<StonePayloadIndexRecord> {
        if self.next_record >= self.header().num_records {
            return None;
        }

        let payload = self.decoded.index()?;
        let record = payload.body.get(self.next_record)?;

        self.next_record += 1;

        Some(record.into())
    }

    pub fn next_attribute_record(&mut self) -> Option<StonePayloadAttributeRecord> {
        if self.next_record >= self.header().num_records {
            return None;
        }

        let payload = self.decoded.attributes()?;
        let record = payload.body.get(self.next_record)?;

        self.next_record += 1;

        Some(record.into())
    }
}

impl From<StoneDecodedPayload> for StonePayload {
    fn from(decoded: StoneDecodedPayload) -> Self {
        Self {
            decoded,
            next_record: 0,
        }
    }
}
