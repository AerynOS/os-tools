// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Read, Seek, SeekFrom, Write};
use thiserror::Error;

use crate::{
    StoneHeader, StoneHeaderV1, StoneHeaderV1FileType, StonePayloadAttributeRecord, StonePayloadCompression,
    StonePayloadEncodeError, StonePayloadHeader, StonePayloadIndexRecord, StonePayloadKind, StonePayloadLayoutRecord,
    StonePayloadMetaRecord, payload,
};

pub use self::digest::{StoneDigestWriter, StoneDigestWriterHasher};

pub mod digest;
mod zstd;

pub struct StoneWriter<W, T> {
    writer: W,
    content: T,
    file_type: StoneHeaderV1FileType,
    payloads: Vec<EncodedPayload>,
    payload_hasher: StoneDigestWriterHasher,
    // TODO: Allow plain encoding?
    encoder: zstd::Encoder,
}

impl<W: Write> StoneWriter<W, ()> {
    pub fn new(writer: W, file_type: StoneHeaderV1FileType) -> Result<Self, StoneWriteError> {
        Ok(Self {
            writer,
            content: (),
            file_type,
            payloads: vec![],
            payload_hasher: StoneDigestWriterHasher::new(),
            encoder: zstd::Encoder::new()?,
        })
    }

    pub fn add_payload<'a>(&mut self, payload: impl Into<StoneWritePayload<'a>>) -> Result<(), StoneWriteError> {
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
    ) -> Result<StoneWriter<W, StoneContentWriter<B>>, StoneWriteError> {
        let mut encoder = zstd::Encoder::new()?;
        encoder.set_pledged_size(pledged_size)?;
        encoder.set_num_workers(num_workers)?;

        Ok(StoneWriter {
            writer: self.writer,
            content: StoneContentWriter {
                buffer,
                plain_size: 0,
                stored_size: 0,
                indices: vec![],
                index_hasher: StoneDigestWriterHasher::new(),
                buffer_hasher: StoneDigestWriterHasher::new(),
                encoder,
            },
            file_type: self.file_type,
            payloads: self.payloads,
            payload_hasher: self.payload_hasher,
            encoder: self.encoder,
        })
    }

    pub fn finalize(mut self) -> Result<(), StoneWriteError> {
        finalize::<_, io::Empty>(&mut self.writer, self.file_type, self.payloads, None)
    }
}

impl<W, B> StoneWriter<W, StoneContentWriter<B>>
where
    W: Write,
    B: Read + Write + Seek,
{
    pub fn add_payload<'a>(&mut self, payload: impl Into<StoneWritePayload<'a>>) -> Result<(), StoneWriteError> {
        self.payloads.push(encode_payload(
            payload.into().into(),
            &mut self.payload_hasher,
            &mut self.encoder,
        )?);
        Ok(())
    }

    pub fn add_content<R: Read>(&mut self, content: &mut R) -> Result<(), StoneWriteError> {
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
            StoneDigestWriter::new(&mut self.content.buffer, &mut self.content.buffer_hasher);
        let mut zstd_writer = zstd::Writer::new(&mut payload_checksum_writer, &mut self.content.encoder);
        let mut index_digest_writer = StoneDigestWriter::new(&mut zstd_writer, &mut self.content.index_hasher);

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
        self.content
            .indices
            .push(StonePayloadIndexRecord { start, end, digest });

        Ok(())
    }

    pub fn finalize(mut self) -> Result<(), StoneWriteError> {
        // Finish frame & get content payload checksum
        let checksum = {
            let mut writer = StoneDigestWriter::new(&mut self.content.buffer, &mut self.content.buffer_hasher);
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

pub struct StoneContentWriter<B> {
    buffer: B,
    plain_size: u64,
    stored_size: u64,
    indices: Vec<StonePayloadIndexRecord>,
    /// Used to generate un-compressed digest of file
    /// contents used for [`Index`]
    index_hasher: StoneDigestWriterHasher,
    /// Used to generate compressed digest of file
    /// contents used for content payload header
    buffer_hasher: StoneDigestWriterHasher,
    encoder: zstd::Encoder,
}

struct EncodedPayload {
    header: StonePayloadHeader,
    content: Vec<u8>,
}

pub enum StoneWritePayload<'a> {
    Meta(&'a [StonePayloadMetaRecord]),
    Attributes(&'a [StonePayloadAttributeRecord]),
    Layout(&'a [StonePayloadLayoutRecord]),
}

impl<'a> From<StoneWritePayload<'a>> for InnerPayload<'a> {
    fn from(payload: StoneWritePayload<'a>) -> Self {
        match payload {
            StoneWritePayload::Meta(payload) => InnerPayload::Meta(payload),
            StoneWritePayload::Attributes(payload) => InnerPayload::Attributes(payload),
            StoneWritePayload::Layout(payload) => InnerPayload::Layout(payload),
        }
    }
}

/// Different from [`Payload`] so public API
/// doesn't support passing in `Index` payloads
/// since it's a side-effect of [`Writer::add_content`]
enum InnerPayload<'a> {
    Meta(&'a [StonePayloadMetaRecord]),
    Attributes(&'a [StonePayloadAttributeRecord]),
    Layout(&'a [StonePayloadLayoutRecord]),
    Index(&'a [StonePayloadIndexRecord]),
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

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StoneWriteError> {
        match self {
            InnerPayload::Meta(records) => payload::encode_records(writer, records)?,
            InnerPayload::Attributes(records) => payload::encode_records(writer, records)?,
            InnerPayload::Layout(records) => payload::encode_records(writer, records)?,
            InnerPayload::Index(records) => payload::encode_records(writer, records)?,
        }
        Ok(())
    }

    fn kind(&self) -> StonePayloadKind {
        match self {
            InnerPayload::Meta(_) => StonePayloadKind::Meta,
            InnerPayload::Attributes(_) => StonePayloadKind::Attributes,
            InnerPayload::Layout(_) => StonePayloadKind::Layout,
            InnerPayload::Index(_) => StonePayloadKind::Index,
        }
    }
}

impl<'a> From<&'a [StonePayloadMetaRecord]> for StoneWritePayload<'a> {
    fn from(payload: &'a [StonePayloadMetaRecord]) -> Self {
        Self::Meta(payload)
    }
}

impl<'a> From<&'a [StonePayloadAttributeRecord]> for StoneWritePayload<'a> {
    fn from(payload: &'a [StonePayloadAttributeRecord]) -> Self {
        Self::Attributes(payload)
    }
}

impl<'a> From<&'a [StonePayloadLayoutRecord]> for StoneWritePayload<'a> {
    fn from(payload: &'a [StonePayloadLayoutRecord]) -> Self {
        Self::Layout(payload)
    }
}

fn encode_payload(
    payload: InnerPayload<'_>,
    hasher: &mut StoneDigestWriterHasher,
    encoder: &mut zstd::Encoder,
) -> Result<EncodedPayload, StoneWriteError> {
    // Reset hasher (it's used across all payloads)
    hasher.reset();
    // Set pledged size
    encoder.set_pledged_size(Some(payload.pledged_size() as u64))?;

    let mut content = vec![];

    // Checksum is on compressed body so we wrap it inside zstd writer
    let mut hashed_writer = StoneDigestWriter::new(&mut content, hasher);
    let mut zstd_writer = zstd::Writer::new(&mut hashed_writer, encoder);

    payload.encode(&mut zstd_writer)?;

    let plain_size = zstd_writer.plain_bytes as u64;

    zstd_writer.finish()?;

    let stored_size = hashed_writer.bytes as u64;

    let header = StonePayloadHeader {
        stored_size,
        plain_size,
        checksum: hasher.digest().to_be_bytes(),
        num_records: payload.num_records(),
        version: 1,
        kind: payload.kind(),
        compression: StonePayloadCompression::Zstd,
    };

    Ok(EncodedPayload { header, content })
}

fn finalize<W: Write, B: Read + Seek>(
    writer: &mut W,
    file_type: StoneHeaderV1FileType,
    payloads: Vec<EncodedPayload>,
    content: Option<(StoneContentWriter<B>, u64)>,
) -> Result<(), StoneWriteError> {
    // Write header
    StoneHeader::V1(StoneHeaderV1 {
        num_payloads: payloads.len() as u16 + if content.is_some() { 1 } else { 0 },
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
        StonePayloadHeader {
            stored_size: content.stored_size,
            plain_size: content.plain_size,
            checksum: checksum.to_be_bytes(),
            num_records: 0,
            version: 1,
            kind: StonePayloadKind::Content,
            compression: StonePayloadCompression::Zstd,
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
pub enum StoneWriteError {
    #[error("payload encode")]
    PayloadEncode(#[from] StonePayloadEncodeError),
    #[error("io")]
    Io(#[from] io::Error),
}
