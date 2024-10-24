// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::missing_safety_doc)]
#![allow(non_camel_case_types)]

use std::{
    fs::File,
    io::{Cursor, Read, Seek},
    os::fd::FromRawFd,
    ptr::NonNull,
    slice,
};

use libc::{c_char, c_int, c_void, size_t};
use stone::{
    StoneDecodedPayload, StoneHeader, StoneHeaderV1, StoneHeaderV1FileType, StoneHeaderVersion,
    StonePayloadCompression, StonePayloadHeader, StonePayloadKind, StonePayloadLayoutFileType,
    StonePayloadMetaDependency, StonePayloadMetaTag,
};

pub use self::payload::{
    StonePayload, StonePayloadAttributeRecord, StonePayloadIndexRecord, StonePayloadLayoutRecord,
    StonePayloadMetaPrimitiveType, StonePayloadMetaRecord,
};

mod payload;

pub const STONE_HEADER_SIZE: usize = 32;

pub type StoneReader<'a> = stone::StoneReader<StoneReadImpl<'a>>;
pub type StonePayloadContentReader<'a> = stone::StonePayloadContentReader<'a, StoneReadImpl<'a>>;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StoneString {
    pub buf: *const u8,
    pub size: size_t,
}

impl StoneString {
    pub fn new(s: &str) -> Self {
        Self {
            buf: s.as_ptr(),
            size: s.len(),
        }
    }
}

pub enum StoneReadImpl<'a> {
    File(File),
    Buffer(Cursor<&'a [u8]>),
    Shim(StoneReadShim),
}

macro_rules! delegate {
    ($self:expr,$($t:tt)*) => {
        match $self {
            StoneReadImpl::File(reader) => reader.$($t)*,
            StoneReadImpl::Buffer(reader) => reader.$($t)*,
            StoneReadImpl::Shim(reader) => reader.$($t)*,
        }
    };
}

impl<'a> Read for StoneReadImpl<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        delegate!(self, read(buf))
    }

    fn read_vectored(&mut self, bufs: &mut [std::io::IoSliceMut<'_>]) -> std::io::Result<usize> {
        delegate!(self, read_vectored(bufs))
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> std::io::Result<usize> {
        delegate!(self, read_to_end(buf))
    }

    fn read_to_string(&mut self, buf: &mut String) -> std::io::Result<usize> {
        delegate!(self, read_to_string(buf))
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        delegate!(self, read_exact(buf))
    }
}

impl<'a> Seek for StoneReadImpl<'a> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        delegate!(self, seek(pos))
    }

    fn rewind(&mut self) -> std::io::Result<()> {
        delegate!(self, rewind())
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        delegate!(self, stream_position())
    }
}

fn fallible(f: impl FnOnce() -> Result<(), Box<dyn std::error::Error>>) -> c_int {
    if f().is_err() { -1 } else { 0 }
}

#[repr(u8)]
enum StoneSeekFrom {
    Start = 0,
    Current = 1,
    End = 2,
}

#[repr(C)]
pub struct StoneReadVTable {
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> usize>,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64, StoneSeekFrom) -> i64>,
}

pub struct StoneReadShim {
    data: *mut c_void,
    read: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> usize,
    seek: unsafe extern "C" fn(*mut c_void, i64, StoneSeekFrom) -> i64,
}

impl Read for StoneReadShim {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        unsafe { Ok((self.read)(self.data, buf.as_mut_ptr() as *mut _, buf.len())) }
    }
}

impl Seek for StoneReadShim {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let (from, offset) = match pos {
            std::io::SeekFrom::Start(i) => (StoneSeekFrom::Start, i as i64),
            std::io::SeekFrom::Current(i) => (StoneSeekFrom::Current, i),
            std::io::SeekFrom::End(i) => (StoneSeekFrom::End, i),
        };

        let ret = unsafe { (self.seek)(self.data, offset, from) };

        if ret < 0 {
            Err(std::io::Error::other(ret.to_string()))
        } else {
            Ok(ret as u64)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_read(
    data: *mut c_void,
    vtable: StoneReadVTable,
    reader_ptr: *mut *mut StoneReader,
    version: *mut StoneHeaderVersion,
) -> c_int {
    fallible(|| unsafe {
        // TODO: Errors
        let data = NonNull::new(data).ok_or("")?;
        let reader_ptr = NonNull::new(reader_ptr).ok_or("")?;
        let mut version = NonNull::new(version).ok_or("")?;

        let reader = stone::read(StoneReadImpl::Shim(StoneReadShim {
            data: data.as_ptr(),
            read: vtable.read.ok_or("")?,
            seek: vtable.seek.ok_or("")?,
        }))?;

        *version.as_mut() = reader.header.version();
        *reader_ptr.as_ptr() = Box::into_raw(Box::new(reader));

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_read_file(
    file: c_int,
    reader_ptr: *mut *mut StoneReader,
    version: *mut StoneHeaderVersion,
) -> c_int {
    fallible(|| unsafe {
        let reader_ptr = NonNull::new(reader_ptr).ok_or("")?;
        let mut version = NonNull::new(version).ok_or("")?;

        let reader = stone::read(StoneReadImpl::File(File::from_raw_fd(file)))?;

        *version.as_mut() = reader.header.version();
        *reader_ptr.as_ptr() = Box::into_raw(Box::new(reader));

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_read_buf(
    buf: *const u8,
    len: usize,
    reader_ptr: *mut *mut StoneReader,
    version: *mut StoneHeaderVersion,
) -> c_int {
    fallible(|| unsafe {
        let buf = NonNull::new(buf as *mut _).ok_or("")?;
        let reader_ptr = NonNull::new(reader_ptr).ok_or("")?;
        let mut version = NonNull::new(version).ok_or("")?;

        let reader = stone::read(StoneReadImpl::Buffer(Cursor::new(slice::from_raw_parts(
            buf.as_ptr(),
            len,
        ))))?;

        *version.as_mut() = reader.header.version();
        *reader_ptr.as_ptr() = Box::into_raw(Box::new(reader));

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_reader_header_v1(reader: *const StoneReader, header: *mut StoneHeaderV1) -> c_int {
    fallible(|| unsafe {
        let reader = NonNull::new(reader as *mut StoneReader).ok_or("")?;
        let mut header = NonNull::new(header).ok_or("")?;

        match &reader.as_ref().header {
            StoneHeader::V1(v1) => *header.as_mut() = *v1,
        }

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_reader_next_payload(
    reader: *mut StoneReader,
    payload_ptr: *mut *mut StonePayload,
) -> c_int {
    fallible(|| unsafe {
        let mut reader = NonNull::new(reader).ok_or("")?;
        let payload_ptr = NonNull::new(payload_ptr).ok_or("")?;

        if let Some(payload) = reader.as_mut().next_payload()? {
            *payload_ptr.as_ptr() = Box::into_raw(Box::new(payload.into()));
        } else {
            Err("no more payloads")?;
        }

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_reader_unpack_content_payload(
    reader: *mut StoneReader,
    payload: *const StonePayload,
    file: c_int,
) -> c_int {
    fallible(|| unsafe {
        let mut reader = NonNull::new(reader).ok_or("")?;
        let payload = NonNull::new(payload as *mut StonePayload).ok_or("")?;
        let mut file = File::from_raw_fd(file);

        if let StoneDecodedPayload::Content(content) = &payload.as_ref().decoded {
            reader.as_mut().unpack_content(content, &mut file)?;
        } else {
            Err("incorrect payload kind")?;
        }

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_reader_read_content_payload<'a>(
    reader: *mut StoneReader<'a>,
    payload: *const StonePayload,
    content_reader: *mut *mut StonePayloadContentReader<'a>,
) -> c_int {
    fallible(|| unsafe {
        let mut reader = NonNull::new(reader).ok_or("")?;
        let payload = NonNull::new(payload as *mut StonePayload).ok_or("")?;
        let content_reader = NonNull::new(content_reader).ok_or("")?;

        if let StoneDecodedPayload::Content(content) = &payload.as_ref().decoded {
            *content_reader.as_ptr() = Box::into_raw(Box::new(reader.as_mut().read_content(content)?));
        } else {
            Err("incorrect payload kind")?;
        }

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_reader_destroy(reader: *mut StoneReader) {
    unsafe {
        let Some(reader) = NonNull::new(reader) else {
            return;
        };

        drop(Box::from_raw(reader.as_ptr()));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_content_reader_read(
    content_reader: *mut StonePayloadContentReader,
    buf: *mut u8,
    size: size_t,
) -> size_t {
    unsafe {
        let Some(mut content_reader) = NonNull::new(content_reader) else {
            return 0;
        };

        content_reader
            .as_mut()
            .read(slice::from_raw_parts_mut(buf, size))
            .unwrap_or(0)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_content_reader_buf_hint(
    content_reader: *const StonePayloadContentReader,
    hint: *mut usize,
) -> c_int {
    fallible(|| unsafe {
        let content_reader = NonNull::new(content_reader as *mut StonePayloadContentReader).ok_or("")?;

        *hint = content_reader.as_ref().buf_hint.unwrap_or(0);

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_content_reader_is_checksum_valid(
    content_reader: *const StonePayloadContentReader,
) -> c_int {
    unsafe {
        let Some(content_reader) = NonNull::new(content_reader as *mut StonePayloadContentReader) else {
            return -1;
        };

        content_reader.as_ref().is_checksum_valid as c_int
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_content_reader_destroy(content_reader: *mut StonePayloadContentReader) {
    unsafe {
        let Some(content_reader) = NonNull::new(content_reader) else {
            return;
        };

        drop(Box::from_raw(content_reader.as_ptr()));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_header(payload: *const StonePayload, header: *mut StonePayloadHeader) -> c_int {
    fallible(|| unsafe {
        let payload = NonNull::new(payload as *mut StonePayload).ok_or("")?;
        let mut header = NonNull::new(header).ok_or("")?;

        *header.as_mut() = *payload.as_ref().header();

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_next_layout_record(
    payload: *mut StonePayload,
    record: *mut StonePayloadLayoutRecord,
) -> c_int {
    fallible(|| unsafe {
        let mut payload = NonNull::new(payload).ok_or("")?;
        let mut record = NonNull::new(record).ok_or("")?;

        *record.as_mut() = payload.as_mut().next_layout_record().ok_or("")?;

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_next_meta_record(
    payload: *mut StonePayload,
    record: *mut StonePayloadMetaRecord,
) -> c_int {
    fallible(|| unsafe {
        let mut payload = NonNull::new(payload).ok_or("")?;
        let mut record = NonNull::new(record).ok_or("")?;

        *record.as_mut() = payload.as_mut().next_meta_record().ok_or("")?;

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_next_index_record(
    payload: *mut StonePayload,
    record: *mut StonePayloadIndexRecord,
) -> c_int {
    fallible(|| unsafe {
        let mut payload = NonNull::new(payload).ok_or("")?;
        let mut record = NonNull::new(record).ok_or("")?;

        *record.as_mut() = payload.as_mut().next_index_record().ok_or("")?;

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_next_attribute_record(
    payload: *mut StonePayload,
    record: *mut StonePayloadAttributeRecord,
) -> c_int {
    fallible(|| unsafe {
        let mut payload = NonNull::new(payload).ok_or("")?;
        let mut record = NonNull::new(record).ok_or("")?;

        *record.as_mut() = payload.as_mut().next_attribute_record().ok_or("")?;

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_payload_destroy(payload: *mut StonePayload) {
    unsafe {
        let Some(payload) = NonNull::new(payload) else {
            return;
        };

        drop(Box::from_raw(payload.as_ptr()));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_header_v1_file_type(file_type: StoneHeaderV1FileType, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, file_type);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_payload_compression(compression: StonePayloadCompression, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, compression);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_payload_kind(kind: StonePayloadKind, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, kind);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_payload_layout_file_type(file_type: StonePayloadLayoutFileType, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, file_type);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_payload_meta_tag(tag: StonePayloadMetaTag, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, tag);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stone_format_payload_meta_dependency(dependency: StonePayloadMetaDependency, buf: *mut u8) {
    unsafe {
        fill_c_string(buf, dependency);
    }
}

unsafe fn fill_c_string(buf: *mut u8, content: impl ToString) {
    unsafe {
        let Some(buf) = NonNull::new(buf) else {
            return;
        };

        let content = content.to_string();
        let buf = slice::from_raw_parts_mut(buf.as_ptr(), content.len() + 1);

        buf[0..content.len()].copy_from_slice(content.as_bytes());
        buf[content.len()] = b'\0';
    }
}
