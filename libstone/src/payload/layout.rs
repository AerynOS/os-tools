// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
use stone::{StonePayloadLayoutFile, StonePayloadLayoutFileType};

use crate::StoneString;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct StonePayloadLayoutRecord {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub tag: u32,
    pub file_type: StonePayloadLayoutFileType,
    pub file_payload: StonePayloadLayoutFilePayload,
}

impl From<&stone::StonePayloadLayoutRecord> for StonePayloadLayoutRecord {
    fn from(record: &stone::StonePayloadLayoutRecord) -> Self {
        StonePayloadLayoutRecord {
            uid: record.uid,
            gid: record.gid,
            mode: record.mode,
            tag: record.tag,
            file_type: record.file.file_type(),
            file_payload: match &record.file {
                StonePayloadLayoutFile::Regular(hash, name) => StonePayloadLayoutFilePayload {
                    regular: StonePayloadLayoutFileRegular {
                        hash: hash.to_be_bytes(),
                        name: StoneString::new(name),
                    },
                },
                StonePayloadLayoutFile::Symlink(source, target) => StonePayloadLayoutFilePayload {
                    symlink: StonePayloadLayoutFileSymlink {
                        source: StoneString::new(source),
                        target: StoneString::new(target),
                    },
                },
                StonePayloadLayoutFile::Directory(name) => StonePayloadLayoutFilePayload {
                    directory: StoneString::new(name),
                },
                StonePayloadLayoutFile::CharacterDevice(name) => StonePayloadLayoutFilePayload {
                    character_device: StoneString::new(name),
                },
                StonePayloadLayoutFile::BlockDevice(name) => StonePayloadLayoutFilePayload {
                    block_device: StoneString::new(name),
                },
                StonePayloadLayoutFile::Fifo(name) => StonePayloadLayoutFilePayload {
                    fifo: StoneString::new(name),
                },
                StonePayloadLayoutFile::Socket(name) => StonePayloadLayoutFilePayload {
                    socket: StoneString::new(name),
                },
                StonePayloadLayoutFile::Unknown(..) => StonePayloadLayoutFilePayload { unknown: () },
            },
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub union StonePayloadLayoutFilePayload {
    regular: StonePayloadLayoutFileRegular,
    symlink: StonePayloadLayoutFileSymlink,
    directory: StoneString,
    character_device: StoneString,
    block_device: StoneString,
    fifo: StoneString,
    socket: StoneString,
    unknown: (),
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct StonePayloadLayoutFileRegular {
    pub hash: [u8; 16],
    pub name: StoneString,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct StonePayloadLayoutFileSymlink {
    pub source: StoneString,
    pub target: StoneString,
}
