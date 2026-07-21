//! Criterion coverage for the shared git-format unified-diff parser.
//!
//! The fixture is intentionally generated here: a checked-in diff large enough to
//! exercise this hot path would be opaque and needlessly inflate the repository.

use std::fmt::Write;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

const FILE_COUNT: usize = 1_200;
const HUNKS_PER_FILE: usize = 3;

fn large_diff() -> String {
    let mut diff = String::with_capacity(FILE_COUNT * HUNKS_PER_FILE * 420);

    for file in 0..FILE_COUNT {
        let path = format!("crates/service_{file:04}/src/component_{file:04}.rs");
        match file % 3 {
            0 => {
                writeln!(diff, "diff --git a/{path} b/{path}").unwrap();
                writeln!(diff, "index 0123456..89abcde 100644").unwrap();
                writeln!(diff, "--- a/{path}").unwrap();
                writeln!(diff, "+++ b/{path}").unwrap();
                for hunk in 0..HUNKS_PER_FILE {
                    let line = hunk * 40 + 1;
                    writeln!(
                        diff,
                        "@@ -{line},4 +{line},4 @@ fn component_{file}_{hunk}() {{"
                    )
                    .unwrap();
                    writeln!(diff, " context_{file}_{hunk}_before();").unwrap();
                    writeln!(diff, "-old_value_{file}_{hunk}();").unwrap();
                    writeln!(diff, "+new_value_{file}_{hunk}();").unwrap();
                    writeln!(diff, " context_{file}_{hunk}_after();").unwrap();
                }
            }
            1 => {
                writeln!(diff, "diff --git a/{path} b/{path}").unwrap();
                writeln!(diff, "new file mode 100644").unwrap();
                writeln!(diff, "index 0000000..89abcde").unwrap();
                writeln!(diff, "--- /dev/null").unwrap();
                writeln!(diff, "+++ b/{path}").unwrap();
                for hunk in 0..HUNKS_PER_FILE {
                    let line = hunk * 4 + 1;
                    writeln!(diff, "@@ -0,0 +{line},3 @@").unwrap();
                    writeln!(diff, "+pub fn added_{file}_{hunk}() {{").unwrap();
                    writeln!(diff, "+    tracing::debug!(\"generated fixture\");").unwrap();
                    writeln!(diff, "+}}").unwrap();
                }
            }
            _ => {
                writeln!(diff, "diff --git a/{path} b/{path}").unwrap();
                writeln!(diff, "deleted file mode 100644").unwrap();
                writeln!(diff, "index 0123456..0000000").unwrap();
                writeln!(diff, "--- a/{path}").unwrap();
                writeln!(diff, "+++ /dev/null").unwrap();
                for hunk in 0..HUNKS_PER_FILE {
                    let line = hunk * 4 + 1;
                    writeln!(diff, "@@ -{line},3 +0,0 @@").unwrap();
                    writeln!(diff, "-fn removed_{file}_{hunk}() {{").unwrap();
                    writeln!(diff, "-    legacy_path_{file}_{hunk}();").unwrap();
                    writeln!(diff, "-}}").unwrap();
                }
            }
        }
    }

    diff
}

fn bench_parse_diff(criterion: &mut Criterion) {
    let fixture = large_diff();
    let mut group = criterion.benchmark_group("diff_parse");
    group.bench_function("1200_files_3_hunks_each", |bencher| {
        bencher.iter(|| vcs_diff::parse_diff(black_box(&fixture)))
    });
    group.finish();
}

criterion_group!(benches, bench_parse_diff);
criterion_main!(benches);
