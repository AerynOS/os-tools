// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Read, Seek, SeekFrom, Write};
use thiserror::Error;

use crate::{
    Header, header,
    payload::{self, Attribute, Index, Layout, Meta},
};

pub mod digest;
mod zstd;

pub struct Writer<W, T = ()> {
    writer: W,
    content: T,
    file_type: header::v1::FileType,
    payloads: Vec<EncodedPayload>,
    payload_hasher: digest::Hasher,
    // TODO: Allow plain encoding?
    encoder: zstd::Encoder,
}

impl<W: Write> Writer<W, ()> {
    pub fn new(writer: W, file_type: header::v1::FileType) -> Result<Self, Error> {
        Ok(Self {
            writer,
            content: (),
            file_type,
            payloads: vec![],
            payload_hasher: digest::Hasher::new(),
            encoder: zstd::Encoder::new()?,
        })
    }

    pub fn add_payload<'a>(&mut self, payload: impl Into<Payload<'a>>) -> Result<(), Error> {
        self.payloads.push(encode_payload(
            payload.into().into(),
            &mut self.payload_hasher,
            &mut self.encoder,
        )?);
        Ok(())
    }

    pub fn with_content<B>(
        self,
        buffer: B,
        pledged_size: Option<u64>,
        num_workers: u32,
    ) -> Result<Writer<W, Content<B>>, Error> {
        let mut encoder = zstd::Encoder::new()?;
        encoder.set_pledged_size(pledged_size)?;
        encoder.set_num_workers(num_workers)?;

        Ok(Writer {
            writer: self.writer,
            content: Content {
                buffer,
                plain_size: 0,
                stored_size: 0,
                indices: vec![],
                index_hasher: digest::Hasher::new(),
                buffer_hasher: digest::Hasher::new(),
                encoder,
            },
            file_type: self.file_type,
            payloads: self.payloads,
            payload_hasher: self.payload_hasher,
            encoder: self.encoder,
        })
    }

    pub fn finalize(mut self) -> Result<(), Error> {
        finalize::<_, io::Empty>(&mut self.writer, self.file_type, self.payloads, None)
    }
}

impl<W, B> Writer<W, Content<B>>
where
    W: Write,
    B: Read + Write + Seek,
{
    pub fn add_payload<'a>(&mut self, payload: impl Into<Payload<'a>>) -> Result<(), Error> {
        self.payloads.push(encode_payload(
            payload.into().into(),
            &mut self.payload_hasher,
            &mut self.encoder,
        )?);
        Ok(())
    }

    pub fn add_content<R: Read>(&mut self, content: &mut R) -> Result<(), Error> {
        // Reset index hasher for this file
        self.content.index_hasher.reset();

        // Start = current plain size
        let start = self.content.plain_size;

        // Compress bytes and output to buffer
        //
        // - Payload checksum is the digest of the compressed bytes across all files
        // - Index digest is the digest of the uncompressed bytes (reset only for this file)
        //
        // Bytes -> index digest -> compression -> buffer checksum -> buffer
        let mut payload_checksum_writer =
            digest::Writer::new(&mut self.content.buffer, &mut self.content.buffer_hasher);
        let mut zstd_writer = zstd::Writer::new(&mut payload_checksum_writer, &mut self.content.encoder);
        let mut index_digest_writer = digest::Writer::new(&mut zstd_writer, &mut self.content.index_hasher);

        io::copy(content, &mut index_digest_writer)?;

        // Add plain bytes
        self.content.plain_size += index_digest_writer.bytes as u64;

        zstd_writer.flush()?;

        // Add compressed bytes
        self.content.stored_size += payload_checksum_writer.bytes as u64;

        // Get digest
        let digest = self.content.index_hasher.digest128();

        // End = current plain size
        let end = self.content.plain_size;

        // Add index data
        self.content.indices.push(Index { start, end, digest });

        Ok(())
    }

    pub fn finalize(mut self) -> Result<(), Error> {
        // Finish frame & get content payload checksum
        let checksum = {
            let mut writer = digest::Writer::new(&mut self.content.buffer, &mut self.content.buffer_hasher);
            self.content.encoder.finish(&mut writer)?;
            writer.flush()?;
            self.content.stored_size += writer.bytes as u64;
            self.content.buffer_hasher.digest()
        };

        // Add index payloads
        self.payloads.push(encode_payload(
            InnerPayload::Index(&self.content.indices),
            &mut self.payload_hasher,
            &mut self.encoder,
        )?);

        finalize(
            &mut self.writer,
            self.file_type,
            self.payloads,
            Some((self.content, checksum)),
        )
    }
}

pub struct Content<B> {
    buffer: B,
    plain_size: u64,
    stored_size: u64,
    indices: Vec<Index>,
    /// Used to generate un-compressed digest of file
    /// contents used for [`Index`]
    index_hasher: digest::Hasher,
    /// Used to generate compressed digest of file
    /// contents used for content payload header
    buffer_hasher: digest::Hasher,
    encoder: zstd::Encoder,
}

struct EncodedPayload {
    header: payload::Header,
    content: Vec<u8>,
}

pub enum Payload<'a> {
    Meta(&'a [Meta]),
    Attributes(&'a [Attribute]),
    Layout(&'a [Layout]),
}

impl<'a> From<Payload<'a>> for InnerPayload<'a> {
    fn from(payload: Payload<'a>) -> Self {
        match payload {
            Payload::Meta(payload) => InnerPayload::Meta(payload),
            Payload::Attributes(payload) => InnerPayload::Attributes(payload),
            Payload::Layout(payload) => InnerPayload::Layout(payload),
        }
    }
}

/// Different from [`Payload`] so public API
/// doesn't support passing in `Index` payloads
/// since it's a side-effect of [`Writer::add_content`]
enum InnerPayload<'a> {
    Meta(&'a [Meta]),
    Attributes(&'a [Attribute]),
    Layout(&'a [Layout]),
    Index(&'a [Index]),
}

impl InnerPayload<'_> {
    fn pledged_size(&self) -> usize {
        match self {
            InnerPayload::Meta(records) => payload::records_total_size(records),
            InnerPayload::Attributes(records) => payload::records_total_size(records),
            InnerPayload::Layout(records) => payload::records_total_size(records),
            InnerPayload::Index(records) => payload::records_total_size(records),
        }
    }

    fn num_records(&self) -> usize {
        match self {
            InnerPayload::Meta(payload) => payload.len(),
            InnerPayload::Attributes(payload) => payload.len(),
            InnerPayload::Layout(payload) => payload.len(),
            InnerPayload::Index(payload) => payload.len(),
        }
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        match self {
            InnerPayload::Meta(records) => payload::encode_records(writer, records)?,
            InnerPayload::Attributes(records) => payload::encode_records(writer, records)?,
            InnerPayload::Layout(records) => payload::encode_records(writer, records)?,
            InnerPayload::Index(records) => payload::encode_records(writer, records)?,
        }
        Ok(())
    }

    fn kind(&self) -> payload::Kind {
        match self {
            InnerPayload::Meta(_) => payload::Kind::Meta,
            InnerPayload::Attributes(_) => payload::Kind::Attributes,
            InnerPayload::Layout(_) => payload::Kind::Layout,
            InnerPayload::Index(_) => payload::Kind::Index,
        }
    }
}

impl<'a> From<&'a [Meta]> for Payload<'a> {
    fn from(payload: &'a [Meta]) -> Self {
        Self::Meta(payload)
    }
}

impl<'a> From<&'a [Attribute]> for Payload<'a> {
    fn from(payload: &'a [Attribute]) -> Self {
        Self::Attributes(payload)
    }
}

impl<'a> From<&'a [Layout]> for Payload<'a> {
    fn from(payload: &'a [Layout]) -> Self {
        Self::Layout(payload)
    }
}

fn encode_payload(
    payload: InnerPayload<'_>,
    hasher: &mut digest::Hasher,
    encoder: &mut zstd::Encoder,
) -> Result<EncodedPayload, Error> {
    // Reset hasher (it's used across all payloads)
    hasher.reset();
    // Set pledged size
    encoder.set_pledged_size(Some(payload.pledged_size() as u64))?;

    let mut content = vec![];

    // Checksum is on compressed body so we wrap it inside zstd writer
    let mut hashed_writer = digest::Writer::new(&mut content, hasher);
    let mut zstd_writer = zstd::Writer::new(&mut hashed_writer, encoder);

    payload.encode(&mut zstd_writer)?;

    let plain_size = zstd_writer.plain_bytes as u64;

    zstd_writer.finish()?;

    let stored_size = hashed_writer.bytes as u64;

    let header = payload::Header {
        stored_size,
        plain_size,
        checksum: hasher.digest().to_be_bytes(),
        num_records: payload.num_records(),
        version: 1,
        kind: payload.kind(),
        compression: payload::Compression::Zstd,
    };

    Ok(EncodedPayload { header, content })
}

fn finalize<W: Write, B: Read + Seek>(
    writer: &mut W,
    file_type: header::v1::FileType,
    payloads: Vec<EncodedPayload>,
    content: Option<(Content<B>, u64)>,
) -> Result<(), Error> {
    // Write header
    Header::V1(header::v1::Header {
        num_payloads: payloads.len() as u16 + content.is_some().then_some(1).unwrap_or_default(),
        file_type,
    })
    .encode(writer)?;

    // Write each payload header + content
    for payload in payloads {
        payload.header.encode(writer)?;
        writer.write_all(&payload.content)?;
    }

    // Write content payload header + buffer
    if let Some((mut content, checksum)) = content {
        payload::Header {
            stored_size: content.stored_size,
            plain_size: content.plain_size,
            checksum: checksum.to_be_bytes(),
            num_records: 0,
            version: 1,
            kind: payload::Kind::Content,
            compression: payload::Compression::Zstd,
        }
        .encode(writer)?;
        // Seek to beginning & copy content buffer
        content.buffer.seek(SeekFrom::Start(0))?;
        io::copy(&mut content.buffer, writer)?;
    }

    writer.flush()?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("payload encode")]
    PayloadEncode(#[from] payload::EncodeError),
    #[error("io")]
    Io(#[from] io::Error),
}
