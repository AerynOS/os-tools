// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use thiserror::Error;

use crate::payload::{Attribute, Compression, Index, Layout, Meta};
use crate::{Header, payload};
use crate::{Payload, header};

use self::zstd::Zstd;

mod digest;
mod zstd;

pub fn read<R: Read + Seek>(mut reader: R) -> Result<Reader<R>, Error> {
    let header = Header::decode(&mut reader).map_err(Error::HeaderDecode)?;

    Ok(Reader {
        header,
        reader,
        hasher: digest::Hasher::new(),
    })
}

pub fn read_bytes(bytes: &[u8]) -> Result<Reader<Cursor<&[u8]>>, Error> {
    read(Cursor::new(bytes))
}

pub struct Reader<R> {
    pub header: Header,
    reader: R,
    hasher: digest::Hasher,
}

impl<R: Read + Seek> Reader<R> {
    pub fn payloads(&mut self) -> Result<impl Iterator<Item = Result<PayloadKind, Error>> + '_, Error> {
        if self.reader.stream_position()? != Header::SIZE as u64 {
            // Rewind to start of payloads
            self.reader.seek(SeekFrom::Start(Header::SIZE as u64))?;
        }

        Ok((0..self.header.num_payloads())
            .flat_map(|_| PayloadKind::decode(&mut self.reader, &mut self.hasher).transpose()))
    }

    pub fn unpack_content<W>(&mut self, content: &Payload<Content>, writer: &mut W) -> Result<(), Error>
    where
        W: Write,
    {
        self.reader.seek(SeekFrom::Start(content.body.offset))?;
        self.hasher.reset();

        let mut hashed = digest::Reader::new(&mut self.reader, &mut self.hasher);
        let mut framed = (&mut hashed).take(content.header.stored_size);
        let mut reader = PayloadReader::new(&mut framed, content.header.compression)?;

        io::copy(&mut reader, writer)?;

        // Validate checksum
        validate_checksum(&self.hasher, &content.header)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Content {
    offset: u64,
}

enum PayloadReader<R: Read> {
    Plain(R),
    Zstd(Zstd<R>),
}

impl<R: Read> PayloadReader<R> {
    fn new(reader: R, compression: Compression) -> Result<Self, Error> {
        Ok(match compression {
            Compression::None => PayloadReader::Plain(reader),
            Compression::Zstd => PayloadReader::Zstd(Zstd::new(reader)?),
        })
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
pub enum PayloadKind {
    Meta(Payload<Vec<Meta>>),
    Attributes(Payload<Vec<Attribute>>),
    Layout(Payload<Vec<Layout>>),
    Index(Payload<Vec<Index>>),
    Content(Payload<Content>),
}

impl PayloadKind {
    fn decode<R: Read + Seek>(mut reader: R, hasher: &mut digest::Hasher) -> Result<Option<Self>, Error> {
        match payload::Header::decode(&mut reader) {
            Ok(header) => {
                hasher.reset();
                let mut hashed = digest::Reader::new(&mut reader, hasher);
                let mut framed = (&mut hashed).take(header.stored_size);

                let payload = match header.kind {
                    payload::Kind::Meta => PayloadKind::Meta(Payload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    payload::Kind::Layout => PayloadKind::Layout(Payload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    payload::Kind::Index => PayloadKind::Index(Payload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    payload::Kind::Attributes => PayloadKind::Attributes(Payload {
                        header,
                        body: payload::decode_records(
                            PayloadReader::new(&mut framed, header.compression)?,
                            header.num_records,
                        )?,
                    }),
                    payload::Kind::Content => {
                        let offset = reader.stream_position()?;

                        // Skip past, these are read by user later
                        reader.seek(SeekFrom::Current(header.stored_size as i64))?;

                        PayloadKind::Content(Payload {
                            header,
                            body: Content { offset },
                        })
                    }
                    payload::Kind::Dumb => unimplemented!("??"),
                };

                // Validate hash for non-content payloads
                if !matches!(header.kind, payload::Kind::Content) {
                    validate_checksum(hasher, &header)?;
                }

                Ok(Some(payload))
            }
            Err(payload::DecodeError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(Error::PayloadDecode(error)),
        }
    }

    pub fn meta(&self) -> Option<&Payload<Vec<Meta>>> {
        if let Self::Meta(meta) = self { Some(meta) } else { None }
    }

    pub fn attributes(&self) -> Option<&Payload<Vec<Attribute>>> {
        if let Self::Attributes(attributes) = self {
            Some(attributes)
        } else {
            None
        }
    }

    pub fn layout(&self) -> Option<&Payload<Vec<Layout>>> {
        if let Self::Layout(layouts) = self {
            Some(layouts)
        } else {
            None
        }
    }

    pub fn index(&self) -> Option<&Payload<Vec<Index>>> {
        if let Self::Index(indices) = self {
            Some(indices)
        } else {
            None
        }
    }

    pub fn content(&self) -> Option<&Payload<Content>> {
        if let Self::Content(content) = self {
            Some(content)
        } else {
            None
        }
    }
}

fn validate_checksum(hasher: &digest::Hasher, header: &payload::Header) -> Result<(), Error> {
    let got = hasher.digest();
    let expected = u64::from_be_bytes(header.checksum);

    if got != expected {
        Err(Error::PayloadChecksum { got, expected })
    } else {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Multiple content payloads not allowed")]
    MultipleContent,
    #[error("header decode")]
    HeaderDecode(#[from] header::DecodeError),
    #[error("payload decode")]
    PayloadDecode(#[from] payload::DecodeError),
    #[error("payload checksum mismatch: got {got:02x}, expected {expected:02x}")]
    PayloadChecksum { got: u64, expected: u64 },
    #[error("payload checksum mismatch: got {got:02x}, expected {expected:02x}, path: {path}")]
    PayloadCheckSumWithContext { got: u64, expected: u64, path: PathBuf },
    #[error("io")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod test {
    use xxhash_rust::xxh3::xxh3_128;

    use crate::payload::layout::Entry;

    use super::*;

    /// Header for bash completion stone archive
    const BASH_TEST_STONE: [u8; 32] = [
        0x0, 0x6d, 0x6f, 0x73, 0x0, 0x4, 0x0, 0x0, 0x1, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0x0, 0x0, 0x4, 0x0, 0x0, 0x5,
        0x0, 0x0, 0x6, 0x0, 0x0, 0x7, 0x1, 0x0, 0x0, 0x0, 0x1,
    ];

    #[test]
    fn read_header() {
        let stone = read_bytes(&BASH_TEST_STONE).expect("valid stone");
        assert_eq!(stone.header.version(), header::Version::V1);
    }

    #[test]
    fn read_bash_completion() {
        let mut stone =
            read_bytes(include_bytes!("../../../../test/bash-completion-2.11-1-1-x86_64.stone")).expect("valid stone");
        assert_eq!(stone.header.version(), header::Version::V1);

        let payloads = stone
            .payloads()
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .expect("seek payloads");

        let mut unpacked_content = vec![];

        if let Some(content) = payloads.iter().find_map(PayloadKind::content) {
            stone
                .unpack_content(content, &mut unpacked_content)
                .expect("valid content");

            for index in payloads.iter().filter_map(PayloadKind::index).flat_map(|p| &p.body) {
                let content = &unpacked_content[index.start as usize..index.end as usize];
                let digest = xxh3_128(content);
                assert_eq!(digest, index.digest);

                payloads
                    .iter()
                    .filter_map(PayloadKind::layout)
                    .flat_map(|p| &p.body)
                    .find(|layout| {
                        if let Entry::Regular(digest, _) = &layout.entry {
                            return *digest == index.digest;
                        }
                        false
                    })
                    .expect("layout exists");
            }
        }
    }
}
