use std::ffi::OsString;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use moss::dynamic_completion::prefix_completer;
use moss::package;

fn criterion_benchmark(c: &mut Criterion) {
    let prefix = OsString::from("lib");

    c.bench_with_input(
        BenchmarkId::new(
            "prefix_completer(package::Flags::default().with_available()",
            prefix.to_string_lossy(),
        ),
        &prefix,
        |b, prefix| {
            b.iter(|| prefix_completer(package::Flags::default().with_available())(prefix));
        },
    );
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
