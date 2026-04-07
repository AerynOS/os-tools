// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    hint::black_box,
    io::{BufReader, Read, Seek, sink},
    path::PathBuf,
};

use criterion::{Criterion, criterion_group, criterion_main};
use fs_err::File;
use stone::StoneDecodedPayload;

fn read_unbuffered(path: impl Into<PathBuf>) {
    read(File::open(path).unwrap());
}

fn read_buffered(path: impl Into<PathBuf>) {
    read(BufReader::new(File::open(path).unwrap()));
}

fn read<R: Read + Seek>(reader: R) {
    let mut stone = stone::read(reader).unwrap();

    let payloads = stone.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();

    if let Some(content) = payloads.iter().find_map(StoneDecodedPayload::content) {
        stone.unpack_content(content, &mut sink()).unwrap();
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("read unbuffered", |b| {
        b.iter(|| read_unbuffered(black_box("../test/bash-completion-2.11-1-1-x86_64.stone")));
    });
    c.bench_function("read buffered", |b| {
        b.iter(|| read_buffered(black_box("../test/bash-completion-2.11-1-1-x86_64.stone")));
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
