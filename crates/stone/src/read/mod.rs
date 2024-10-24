// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
#![allow(dead_code)]

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use thiserror::Error;

use crate::{
    StoneHeader, StoneHeaderDecodeError, StonePayload, StonePayloadAttributeRecord, StonePayloadCompression,
    StonePayloadContent, StonePayloadDecodeError, StonePayloadHeader, StonePayloadIndexRecord, StonePayloadKind,
    StonePayloadLayoutRecord, StonePayloadMetaRecord, payload,
};

use self::zstd::Zstd;

mod digest;
mod zstd;

pub fn read<R: Read + Seek>(mut reader: R) -> Result<StoneReader<R>, StoneReadError> {
    let header = StoneHeader::decode(&mut reader).map_err(StoneReadError::HeaderDecode)?;

    Ok(StoneReader {
        header,
        reader,
        hasher: digest::Hasher::new(),

        #[cfg(feature = "ffi")]
        next_payload: 0,
    })
}

pub fn read_bytes(bytes: &[u8]) -> Result<StoneReader<Cursor<&[u8]>>, StoneReadError> {
    read(Cursor::new(bytes))
}

pub struct StoneReader<R> {
    pub header: StoneHeader,
    reader: R,
    hasher: digest::Hasher,

    #[cfg(feature = "ffi")]
    next_payload: u16,
}

impl<R: Read + Seek> StoneReader<R> {
    pub fn payloads(
        &mut self,
    ) -> Result<impl Iterator<Item = Result<StoneDecodedPayload, StoneReadError>> + '_, StoneReadError> {
        if self.reader.stream_position()? != StoneHeader::SIZE as u64 {
            // Rewind to start of payloads
            self.reader.seek(SeekFrom::Start(StoneHeader::SIZE as u64))?;
        }

        #[cfg(feature = "ffi")]
        {
            self.next_payload = self.header.num_payloads();
        }

        Ok((0..self.header.num_payloads())
            .flat_map(|_| StoneDecodedPayload::decode(&mut self.reader, &mut self.hasher).transpose()))
    }

    pub fn unpack_content<W>(
        &mut self,
        content: &StonePayload<StonePayloadContent>,
        writer: &mut W,
    ) -> Result<(), StoneReadError>
    where
        W: Write,
    {
        self.reader.seek(SeekFrom::Start(content.body.offset))?;
        self.hasher.reset();

        let hashed = digest::Reader::new(&mut self.reader, &mut self.hasher);
        let framed = hashed.take(content.header.stored_size);

        io::copy(&mut PayloadReader::new(framed, content.header.compression)?, writer)?;

        // Validate checksum
        validate_checksum(&self.hasher, &content.header)?;

        Ok(())
    }
}

#[cfg(feature = "ffi")]
impl<R: Read + Seek> StoneReader<R> {
    pub fn next_payload(&mut self) -> Result<Option<StoneDecodedPayload>, StoneReadError> {
        if self.next_payload < self.header.num_payloads() {
            let payload = StoneDecodedPayload::decode(&mut self.reader, &mut self.hasher)?;

            self.next_payload += 1;

            Ok(payload)
        } else {
            Ok(None)
        }
    }

    pub fn read_content<'a>(
        &'a mut self,
        content: &StonePayload<StonePayloadContent>,
    ) -> Result<StonePayloadContentReader<'a, R>, StoneReadError> {
        self.reader.seek(SeekFrom::Start(content.body.offset))?;
        self.hasher.reset();

        let hashed = digest::Reader::new(&mut self.reader, &mut self.hasher);
        let framed = hashed.take(content.header.stored_size);
        let reader = PayloadReader::new(framed, content.header.compression)?;

        let buf_hint = reader.buf_hint();

        Ok(StonePayloadContentReader {
            reader,
            header_checksum: u64::from_be_bytes(content.header.checksum),
            is_checksum_valid: false,
            buf_hint,
        })
    }
}

#[cfg(feature = "ffi")]
pub struct StonePayloadContentReader<'a, R: Read> {
    reader: PayloadReader<io::Take<digest::Reader<'a, &'a mut R>>>,
    header_checksum: u64,
    pub is_checksum_valid: bool,
    pub buf_hint: Option<usize>,
}

#[cfg(feature = "ffi")]
impl<'a, R: Read> Read for StonePayloadContentReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.reader.read(buf) {
            Ok(read) if !buf.is_empty() && read == 0 => {
                self.is_checksum_valid = self.header_checksum == self.reader.get_mut().get_mut().hasher.digest();

                Ok(read)
            }
            Ok(read) => Ok(read),
            e @ Err(_) => e,
        }
    }
}

enum PayloadReader<R: Read> {
    Plain(R),
    Zstd(Zstd<R>),
}

impl<R: Read> PayloadReader<R> {
    fn new(reader: R, compression: StonePayloadCompression) -> Result<Self, StoneReadError> {
        Ok(match compression {
            StonePayloadCompression::None => PayloadReader::Plain(reader),
            StonePayloadCompression::Zstd => PayloadReader::Zstd(Zstd::new(reader)?),
            StonePayloadCompression::Unknown => return Err(StoneReadError::UnknownCompression),
        })
    }

    fn get_mut(&mut self) -> &mut R {
        match self {
            PayloadReader::Plain(reader) => reader,
            PayloadReader::Zstd(reader) => reader.get_mut(),
        }
    }

    fn buf_hint(&self) -> Option<usize> {
        match self {
            PayloadReader::Plain(_) => None,
            PayloadReader::Zstd(z) => Some(z.capacity()),
        }
    }
}

impl<R: Read> Read for PayloadReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            PayloadReader::Plain(reader) => reader.read(buf),
            PayloadReader::Zstd(reader) => reader.read(buf),
        }
    }
}

#[derive(Debug)]
pub enum StoneDecodedPayload {
    Meta(StonePayload<Vec<StonePayloadMetaRecord>>),
    Attributes(StonePayload<Vec<StonePayloadAttributeRecord>>),
    Layout(StonePayload<Vec<StonePayloadLayoutRecord>>),
    Index(StonePayload<Vec<StonePayloadIndexRecord>>),
    Content(StonePayload<StonePayloadContent>),

    /// Payload type not known / supported by this decoder
    Unknown(StonePayload<()>),
    /// Payload compression type not known / supported by this decoder
    UnknownCompression(StonePayload<()>),
}

impl StoneDecodedPayload {
    pub fn header(&self) -> &StonePayloadHeader {
        match self {
            StoneDecodedPayload::Meta(payload) => &payload.header,
            StoneDecodedPayload::Attributes(payload) => &payload.header,
            StoneDecodedPayload::Layout(payload) => &payload.header,
            StoneDecodedPayload::Index(payload) => &payload.header,
            StoneDecodedPayload::Content(payload) => &payload.header,
            StoneDecodedPayload::Unknown(payload) => &payload.header,
            StoneDecodedPayload::UnknownCompression(payload) => &payload.header,
        }
    }

    fn decode<R: Read + Seek>(mut reader: R, hasher: &mut digest::Hasher) -> Result<Option<Self>, StoneReadError> {
        match StonePayloadHeader::decode(&mut reader) {
            Ok(header) => {
                hasher.reset();
                let mut hashed = digest::Reader::new(&mut reader, hasher);
                let mut framed = (&mut hashed).take(header.stored_size);

                // Don't try to decode if unknown compression (we can't) & instead skip this payload
                // so we can continue decoding
                if matches!(header.compression, StonePayloadCompression::Unknown) {
                    reader.seek(SeekFrom::Current(header.stored_size as i64))?;

                    return Ok(Some(StoneDecodedPayload::UnknownCompression(StonePayload {
                        header,
                        body: (),
                    })));
                }

                let payload = match header.kind {
                    StonePayloadKind::Meta => StoneDecodedPayload::Meta(StonePayload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    StonePayloadKind::Layout => StoneDecodedPayload::Layout(StonePayload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    StonePayloadKind::Index => StoneDecodedPayload::Index(StonePayload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    StonePayloadKind::Attributes => StoneDecodedPayload::Attributes(StonePayload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    StonePayloadKind::Content => {
                        // Skip past, these are read by user later
                        let new_offset = reader.seek(SeekFrom::Current(header.stored_size as i64))?;

                        StoneDecodedPayload::Content(StonePayload {
                            header,
                            body: StonePayloadContent {
                                offset: (new_offset as i64 - header.stored_size as i64) as u64,
                            },
                        })
                    }
                    StonePayloadKind::Unknown => {
                        // Skip past, we don't know how to decode this
                        reader.seek(SeekFrom::Current(header.stored_size as i64))?;

                        StoneDecodedPayload::Unknown(StonePayload { header, body: () })
                    }
                };

                // Validate hash for known, non-content payloads
                if !matches!(header.kind, StonePayloadKind::Content | StonePayloadKind::Unknown) {
                    validate_checksum(hasher, &header)?;
                }

                Ok(Some(payload))
            }
            Err(StonePayloadDecodeError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(StoneReadError::PayloadDecode(error)),
        }
    }

    pub fn meta(&self) -> Option<&StonePayload<Vec<StonePayloadMetaRecord>>> {
        if let Self::Meta(meta) = self { Some(meta) } else { None }
    }

    pub fn attributes(&self) -> Option<&StonePayload<Vec<StonePayloadAttributeRecord>>> {
        if let Self::Attributes(attributes) = self {
            Some(attributes)
        } else {
            None
        }
    }

    pub fn layout(&self) -> Option<&StonePayload<Vec<StonePayloadLayoutRecord>>> {
        if let Self::Layout(layouts) = self {
            Some(layouts)
        } else {
            None
        }
    }

    pub fn index(&self) -> Option<&StonePayload<Vec<StonePayloadIndexRecord>>> {
        if let Self::Index(indices) = self {
            Some(indices)
        } else {
            None
        }
    }

    pub fn content(&self) -> Option<&StonePayload<StonePayloadContent>> {
        if let Self::Content(content) = self {
            Some(content)
        } else {
            None
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            StoneDecodedPayload::Meta(_) => "Meta",
            StoneDecodedPayload::Attributes(_) => "Attributes",
            StoneDecodedPayload::Layout(_) => "Layout",
            StoneDecodedPayload::Index(_) => "Index",
            StoneDecodedPayload::Content(_) => "Content",
            StoneDecodedPayload::Unknown(_) => "Unknown payload type",
            StoneDecodedPayload::UnknownCompression(payload) => match payload.header.kind {
                StonePayloadKind::Unknown => "Unknown payload type & compression",
                StonePayloadKind::Meta => "Meta - unknown compression",
                StonePayloadKind::Content => "Content - unknown compression",
                StonePayloadKind::Layout => "Layout - unknown compression",
                StonePayloadKind::Index => "Index - unknown compression",
                StonePayloadKind::Attributes => "Attributes - unknown compression",
            },
        }
    }
}

fn validate_checksum(hasher: &digest::Hasher, header: &StonePayloadHeader) -> Result<(), StoneReadError> {
    let got = hasher.digest();
    let expected = u64::from_be_bytes(header.checksum);

    if got != expected {
        Err(StoneReadError::PayloadChecksum { got, expected })
    } else {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum StoneReadError {
    #[error("Multiple content payloads not allowed")]
    MultipleContent,
    #[error("Unknown payload compression")]
    UnknownCompression,
    #[error("header decode")]
    HeaderDecode(#[from] StoneHeaderDecodeError),
    #[error("payload decode")]
    PayloadDecode(#[from] StonePayloadDecodeError),
    #[error("payload checksum mismatch: got {got:02x}, expected {expected:02x}")]
    PayloadChecksum { got: u64, expected: u64 },
    #[error("io")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod test {
    use xxhash_rust::xxh3::xxh3_128;

    use crate::{StoneHeaderVersion, StonePayloadLayoutFile};

    use super::*;

    /// Header for bash completion stone archive
    const BASH_TEST_STONE: [u8; 32] = [
        0x0, 0x6d, 0x6f, 0x73, 0x0, 0x4, 0x0, 0x0, 0x1, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0x0, 0x0, 0x4, 0x0, 0x0, 0x5,
        0x0, 0x0, 0x6, 0x0, 0x0, 0x7, 0x1, 0x0, 0x0, 0x0, 0x1,
    ];

    #[test]
    fn read_header() {
        let stone = read_bytes(&BASH_TEST_STONE).expect("valid stone");
        assert_eq!(stone.header.version(), StoneHeaderVersion::V1);
    }

    #[test]
    fn read_bash_completion() {
        let mut stone =
            read_bytes(include_bytes!("../../../../test/bash-completion-2.11-1-1-x86_64.stone")).expect("valid stone");
        assert_eq!(stone.header.version(), StoneHeaderVersion::V1);

        let payloads = stone
            .payloads()
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .expect("seek payloads");

        let mut unpacked_content = vec![];

        if let Some(content) = payloads.iter().find_map(StoneDecodedPayload::content) {
            stone
                .unpack_content(content, &mut unpacked_content)
                .expect("valid content");

            for index in payloads
                .iter()
                .filter_map(StoneDecodedPayload::index)
                .flat_map(|p| &p.body)
            {
                let content = &unpacked_content[index.start as usize..index.end as usize];
                let digest = xxh3_128(content);
                assert_eq!(digest, index.digest);

                payloads
                    .iter()
                    .filter_map(StoneDecodedPayload::layout)
                    .flat_map(|p| &p.body)
                    .find(|layout| {
                        if let StonePayloadLayoutFile::Regular(digest, _) = &layout.file {
                            return *digest == index.digest;
                        }
                        false
                    })
                    .expect("layout exists");
            }
        }
    }
}
