// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

pub(crate) mod ext;
mod header;
mod payload;
mod read;
mod write;

pub use self::header::{
    STONE_HEADER_MAGIC, StoneAgnosticHeader, StoneHeader, StoneHeaderDecodeError, StoneHeaderV1,
    StoneHeaderV1DecodeError, StoneHeaderV1FileType, StoneHeaderVersion,
};
pub use self::payload::{
    StonePayload, StonePayloadAttributeRecord, StonePayloadCompression, StonePayloadContent, StonePayloadDecodeError,
    StonePayloadEncodeError, StonePayloadHeader, StonePayloadIndexRecord, StonePayloadKind, StonePayloadLayoutFile,
    StonePayloadLayoutFileType, StonePayloadLayoutRecord, StonePayloadMetaDependency, StonePayloadMetaPrimitive,
    StonePayloadMetaRecord, StonePayloadMetaTag,
};
#[cfg(feature = "ffi")]
pub use self::read::StonePayloadContentReader;
pub use self::read::{StoneDecodedPayload, StoneReadError, StoneReader, read, read_bytes};
pub use self::write::{
    StoneContentWriter, StoneDigestWriter, StoneDigestWriterHasher, StoneWriteError, StoneWritePayload, StoneWriter,
};

#[cfg(test)]
mod test {
    use std::{io::Cursor, thread};

    use super::*;

    #[test]
    fn roundtrip() {
        let in_stone = include_bytes!("../../../test/bash-completion-2.11-1-1-x86_64.stone");

        let mut reader = read_bytes(in_stone).unwrap();

        let payloads = reader.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let meta = payloads.iter().find_map(StoneDecodedPayload::meta).unwrap();
        let layouts = payloads.iter().find_map(StoneDecodedPayload::layout).unwrap();
        let indices = payloads.iter().find_map(StoneDecodedPayload::index).unwrap();
        let content = payloads.iter().find_map(StoneDecodedPayload::content).unwrap();

        let mut content_buffer = vec![];

        reader.unpack_content(content, &mut content_buffer).unwrap();

        let mut out_stone = vec![];
        let mut temp_content_buffer: Vec<u8> = vec![];
        let mut writer = StoneWriter::new(&mut out_stone, StoneHeaderV1FileType::Binary)
            .unwrap()
            .with_content(
                Cursor::new(&mut temp_content_buffer),
                Some(content_buffer.len() as u64),
                thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as u32,
            )
            .unwrap();

        writer.add_payload(meta.body.as_slice()).unwrap();

        for index in &indices.body {
            let mut bytes = &content_buffer[index.start as usize..index.end as usize];

            writer.add_content(&mut bytes).unwrap();
        }

        // We'd typically add layouts after calling `add_content` since
        // we will determine the layout when processing the file during
        // that iteration
        writer.add_payload(layouts.body.as_slice()).unwrap();

        writer.finalize().unwrap();

        let mut rt_reader = read_bytes(&out_stone).unwrap();
        assert_eq!(rt_reader.header, reader.header);

        let rt_payloads = rt_reader.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let rt_meta = rt_payloads.iter().find_map(StoneDecodedPayload::meta).unwrap();
        let rt_layouts = rt_payloads.iter().find_map(StoneDecodedPayload::layout).unwrap();
        let rt_indices = rt_payloads.iter().find_map(StoneDecodedPayload::index).unwrap();
        let rt_content = rt_payloads.iter().find_map(StoneDecodedPayload::content).unwrap();

        // Stored size / digest will be different since compression from boulder
        // isn't identical & we don't add null terminated strings
        assert_eq!(rt_indices.header.plain_size, indices.header.plain_size);
        assert_eq!(rt_content.header.plain_size, content.header.plain_size);
        assert_eq!(rt_meta.body.len(), meta.body.len());
        assert_eq!(rt_layouts.body.len(), layouts.body.len());

        assert!(meta.body.iter().zip(&rt_meta.body).all(|(a, b)| a == b));
        assert!(layouts.body.iter().zip(&rt_layouts.body).all(|(a, b)| a == b));
        assert!(indices.body.iter().zip(&rt_indices.body).all(|(a, b)| a == b));

        let mut rt_content_buffer = vec![];

        rt_reader.unpack_content(rt_content, &mut rt_content_buffer).unwrap();

        assert_eq!(rt_content_buffer, content_buffer);

        println!(
            "Boulder-D stone size => {}, stone-rs stone size => {}",
            in_stone.len(),
            out_stone.len()
        );
    }
}
