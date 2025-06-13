// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{self, Result, Write};

use zstd::zstd_safe::zstd_sys::ZSTD_EndDirective;
use zstd::zstd_safe::{self, InBuffer, OutBuffer};
use zstd::zstd_safe::{CParameter, ResetDirective};

type Context = zstd_safe::CCtx<'static>;

/// Transparent encapsulation of zstd compression with the purpose
/// of encoding moss (.stone) payloads to a stream
pub struct Writer<'a, W: Write> {
    writer: W,
    encoder: &'a mut Encoder,
    pub plain_bytes: usize,
}

impl<'a, W: Write> Writer<'a, W> {
    /// Construct a new Writerfor the given writer
    pub fn new(writer: W, encoder: &'a mut Encoder) -> Self {
        Self {
            writer,
            encoder,
            plain_bytes: 0,
        }
    }

    /// Finish a frame to this writer
    pub fn finish(mut self) -> Result<()> {
        self.encoder.finish(&mut self.writer)?;
        self.writer.flush()?;
        Ok(())
    }
}

impl<W: Write> Write for Writer<'_, W> {
    /// Handle transparent encoding, record offsets
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.plain_bytes += buf.len();

        let mut input = InBuffer::around(&buf[0..usize::min(self.encoder.read_size, buf.len())]);
        let mut finished = false;

        while !finished {
            let mut output_buffer = OutBuffer::around(&mut self.encoder.output);

            let _status = self
                .encoder
                .context
                .compress_stream2(&mut output_buffer, &mut input, ZSTD_EndDirective::ZSTD_e_continue)
                .map_err(map_error_code)?;

            self.writer.write_all(output_buffer.as_slice())?;

            finished = input.pos() == input.src.len();
        }

        Ok(input.pos)
    }

    /// Flush the stream
    fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }
}

pub struct Encoder {
    context: Context,
    output: Vec<u8>,
    read_size: usize,
}

impl Encoder {
    /// Concrete zstd encoder
    pub fn new() -> Result<Self> {
        let mut context = Context::create();
        context
            .set_parameter(CParameter::CompressionLevel(18))
            .map_err(map_error_code)?;
        context
            .set_parameter(CParameter::WindowLog(31))
            .map_err(map_error_code)?;
        Ok(Self {
            context,
            output: vec![0; Context::out_size()],
            read_size: Context::in_size(),
        })
    }

    /// Let zstd know of the final uncompressed size, to optimise compression
    pub fn set_pledged_size(&mut self, pledged_size: Option<u64>) -> Result<()> {
        self.context
            .set_pledged_src_size(pledged_size)
            .map_err(map_error_code)?;
        Ok(())
    }

    pub fn set_num_workers(&mut self, num_workers: u32) -> Result<()> {
        self.context
            .set_parameter(CParameter::NbWorkers(num_workers))
            .map_err(map_error_code)?;
        Ok(())
    }

    /// Manually finish a frame to the provided writer
    pub fn finish<W: Write>(&mut self, writer: &mut W) -> Result<()> {
        let mut finished = false;

        while !finished {
            let mut output_buffer = OutBuffer::around(&mut self.output);

            let remaining = self
                .context
                .compress_stream2(
                    &mut output_buffer,
                    &mut InBuffer::around(&[]),
                    ZSTD_EndDirective::ZSTD_e_end,
                )
                .map_err(map_error_code)?;

            writer.write_all(output_buffer.as_slice())?;

            finished = remaining == 0;
        }

        self.context
            .reset(ResetDirective::SessionOnly)
            .map_err(map_error_code)?;
        self.context.set_pledged_src_size(None).map_err(map_error_code)?;

        Ok(())
    }
}

fn map_error_code(code: usize) -> io::Error {
    let msg = zstd_safe::get_error_name(code);
    io::Error::other(msg.to_owned())
}
