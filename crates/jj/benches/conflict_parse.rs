//! Criterion coverage for jj's native snapshot-style conflict parser.

use std::fmt::Write;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

const CONFLICT_COUNT: usize = 600;

fn large_snapshot_conflicted_file() -> String {
    let mut content = String::with_capacity(CONFLICT_COUNT * 390);

    for conflict in 1..=CONFLICT_COUNT {
        writeln!(content, "// surrounding declaration {conflict}").unwrap();
        writeln!(content, "<<<<<<< conflict {conflict} of {CONFLICT_COUNT}").unwrap();
        writeln!(content, "+++++++ side-a-{conflict:04} 01234567 \"main\"").unwrap();
        writeln!(content, "main_value_{conflict}();").unwrap();
        writeln!(content, "------- base-{conflict:04} 89abcdef \"base\"").unwrap();
        writeln!(content, "base_value_{conflict}();").unwrap();
        writeln!(content, "+++++++ side-b-{conflict:04} fedcba98 \"feature\"").unwrap();
        writeln!(content, "feature_value_{conflict}();").unwrap();
        writeln!(
            content,
            ">>>>>>> conflict {conflict} of {CONFLICT_COUNT} ends"
        )
        .unwrap();
    }

    content
}

fn bench_parse_conflicts(criterion: &mut Criterion) {
    let fixture = large_snapshot_conflicted_file();
    let mut group = criterion.benchmark_group("jj_conflict_parse");
    group.bench_function("600_snapshot_regions", |bencher| {
        bencher.iter(|| {
            vcs_jj::conflict::parse_conflicts(black_box(&fixture)).expect("fixture is valid")
        })
    });
    group.bench_function("600_snapshot_regions_parse_and_render_exact", |bencher| {
        bencher.iter(|| {
            let segments =
                vcs_jj::conflict::parse_conflicts(black_box(&fixture)).expect("fixture is valid");
            let rendered = vcs_jj::conflict::render(black_box(&segments));
            assert_eq!(rendered, fixture, "render must remain byte-exact");
            rendered
        })
    });
    group.finish();
}

criterion_group!(benches, bench_parse_conflicts);
criterion_main!(benches);
