// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::Read;

use xxhash_rust::xxh3::Xxh3;

pub type Hasher = Xxh3;

pub struct Reader<'a, R: Read> {
    inner: R,
    pub hasher: &'a mut Hasher,
}

impl<'a, R> Reader<'a, R>
where
    R: Read,
{
    pub fn new(reader: R, hasher: &'a mut Hasher) -> Self {
        Self { inner: reader, hasher }
    }
}

impl<R> Read for Reader<'_, R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}
