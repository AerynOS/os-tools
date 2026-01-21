// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::Write;

use xxhash_rust::xxh3::Xxh3;

pub type StoneDigestWriterHasher = Xxh3;

pub struct StoneDigestWriter<'a, W: Write> {
    inner: W,
    hasher: &'a mut StoneDigestWriterHasher,
    pub bytes: usize,
}

impl<'a, W> StoneDigestWriter<'a, W>
where
    W: Write,
{
    pub fn new(writer: W, hasher: &'a mut StoneDigestWriterHasher) -> Self {
        Self {
            inner: writer,
            hasher,
            bytes: 0,
        }
    }
}

impl<W> Write for StoneDigestWriter<'_, W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len();
        self.hasher.update(buf);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
