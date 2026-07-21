//! Criterion coverage for NUL-delimited `git status --porcelain=v2 --branch`.
//!
//! The fixture mixes ordinary edits, renames, unresolved conflicts, untracked
//! files, and ignored build artifacts, as a large monorepo status commonly does.

use std::fmt::Write;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

const RECORD_COUNT: usize = 2_500;

fn large_porcelain_v2() -> String {
    let mut output = String::with_capacity(RECORD_COUNT * 130);
    output.push_str("# branch.oid 0123456789abcdef0123456789abcdef01234567\0");
    output.push_str("# branch.head feature/parser-throughput\0");
    output.push_str("# branch.upstream origin/feature/parser-throughput\0");
    output.push_str("# branch.ab +17 -4\0");

    for record in 0..RECORD_COUNT {
        let path = format!(
            "crates/workspace_{:04}/src/module_{:04}.rs",
            record % 400,
            record
        );
        match record % 5 {
            0 => write!(
                output,
                "1 M. N... 100644 100644 100644 0123456 89abcde {path}\0"
            )
            .unwrap(),
            1 => {
                let old_path = format!(
                    "crates/workspace_{:04}/src/legacy_{:04}.rs",
                    record % 400,
                    record
                );
                write!(
                    output,
                    "2 R. N... 100644 100644 100644 0123456 89abcde R100 {path}\0{old_path}\0"
                )
                .unwrap();
            }
            2 => write!(
                output,
                "u UU N... 100644 100644 100644 100644 0123456 89abcde fedcba9 {path}\0"
            )
            .unwrap(),
            3 => write!(output, "? generated/cache_{record:04}/artifact.json\0").unwrap(),
            _ => write!(output, "! target/workspace_{record:04}/incremental.bin\0").unwrap(),
        }
    }

    output
}

fn bench_parse_porcelain_v2(criterion: &mut Criterion) {
    let fixture = large_porcelain_v2();
    let mut group = criterion.benchmark_group("porcelain_v2");
    group.bench_function("2500_mixed_records", |bencher| {
        bencher.iter(|| vcs_git::parse_porcelain_v2(black_box(&fixture)))
    });
    group.finish();
}

criterion_group!(benches, bench_parse_porcelain_v2);
criterion_main!(benches);
