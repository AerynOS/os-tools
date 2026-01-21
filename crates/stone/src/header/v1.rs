// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use thiserror::Error;

/// Simple corruption check in the header, will be expanded for v2
const INTEGRITY_CHECK: [u8; 21] = [0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0, 4, 0, 0, 5, 0, 0, 6, 0, 0, 7];

/// Well known file type for a v1 stone container
///
/// Some types are now legacy as we're going to use Ion to define them.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
#[repr(u8)]
pub enum StoneHeaderV1FileType {
    /// Binary package
    Binary = 1,

    /// Delta package
    Delta = 2,

    /// (Legacy) repository index
    Repository = 3,

    /// (Legacy) build manifest
    BuildManifest = 4,

    Unknown = 255,
}

/// Header for the v1 format version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct StoneHeaderV1 {
    pub num_payloads: u16,
    pub file_type: StoneHeaderV1FileType,
}

impl StoneHeaderV1 {
    pub fn decode(bytes: [u8; 24]) -> Result<Self, StoneHeaderV1DecodeError> {
        let (num_payloads, rest) = bytes.split_at(2);
        let (integrity_check, file_type) = rest.split_at(21);

        if integrity_check != INTEGRITY_CHECK {
            return Err(StoneHeaderV1DecodeError::Corrupt);
        }

        let num_payloads = u16::from_be_bytes(num_payloads.try_into().unwrap());
        let file_type = match file_type[0] {
            1 => StoneHeaderV1FileType::Binary,
            2 => StoneHeaderV1FileType::Delta,
            3 => StoneHeaderV1FileType::Repository,
            4 => StoneHeaderV1FileType::BuildManifest,
            _ => StoneHeaderV1FileType::Unknown,
        };

        Ok(Self {
            num_payloads,
            file_type,
        })
    }

    pub fn encode(&self) -> [u8; 24] {
        let mut data = [0u8; 24];

        let num_payloads = u16::to_be_bytes(self.num_payloads);
        let file_type = self.file_type as u8;

        data[0..2].copy_from_slice(&num_payloads);
        data[2..23].copy_from_slice(&INTEGRITY_CHECK);
        data[23] = file_type;

        data
    }
}

#[derive(Debug, Error)]
pub enum StoneHeaderV1DecodeError {
    #[error("Corrupt header, failed integrity check")]
    Corrupt,
}
