// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::File,
    io::{sink, BufReader, Read, Seek},
    path::Path,
};

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn read_unbuffered(path: impl AsRef<Path>) {
    read(File::open(path).unwrap());
}

fn read_buffered(path: impl AsRef<Path>) {
    read(BufReader::new(File::open(path).unwrap()));
}

fn read<R: Read + Seek>(reader: R) {
    let mut stone = stone::read(reader).unwrap();

    let payloads = stone.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();

    if let Some(content) = payloads.iter().find_map(stone::read::PayloadKind::content) {
        stone.unpack_content(content, &mut sink()).unwrap();
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("read unbuffered", |b| {
        b.iter(|| read_unbuffered(black_box("../test/bash-completion-2.11-1-1-x86_64.stone")))
    });
    c.bench_function("read buffered", |b| {
        b.iter(|| read_buffered(black_box("../test/bash-completion-2.11-1-1-x86_64.stone")))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
