// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{BufReader, Read, Result};

use zstd::stream::read::Decoder;

pub struct Zstd<R: Read> {
    decoder: Decoder<'static, BufReader<R>>,
}

impl<R: Read> Zstd<R> {
    pub fn new(reader: R) -> Result<Self> {
        let mut decoder = Decoder::new(reader)?;
        decoder.window_log_max(31)?;

        Ok(Self { decoder })
    }

    pub fn get_mut(&mut self) -> &mut R {
        self.decoder.get_mut().get_mut()
    }

    pub fn capacity(&self) -> usize {
        self.decoder.get_ref().capacity()
    }
}

impl<R: Read> Read for Zstd<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.decoder.read(buf)
    }
}
