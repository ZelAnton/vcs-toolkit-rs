//! Criterion coverage for git conflict parsing and its byte-exact renderer.

use std::fmt::Write;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

const CONFLICT_COUNT: usize = 600;

fn large_conflicted_file() -> String {
    let mut content = String::with_capacity(CONFLICT_COUNT * 360);

    for conflict in 1..=CONFLICT_COUNT {
        let marker = if conflict % 4 == 0 {
            "<<<<<<<<<"
        } else {
            "<<<<<<<"
        };
        let base_marker = marker.replace('<', "|");
        let separator = marker.replace('<', "=");
        let end_marker = marker.replace('<', ">");
        let ending = if conflict % 10 == 0 { "\r\n" } else { "\n" };

        writeln!(content, "// surrounding declaration {conflict}").unwrap();
        write!(content, "{marker} HEAD{ending}").unwrap();
        write!(content, "ours_value_{conflict}();{ending}").unwrap();
        if conflict % 2 == 0 {
            write!(content, "{base_marker} merge-base{ending}").unwrap();
            write!(content, "base_value_{conflict}();{ending}").unwrap();
        }
        write!(content, "{separator}{ending}").unwrap();
        write!(content, "theirs_value_{conflict}();{ending}").unwrap();
        write!(content, "{end_marker} feature/parallel-{conflict}{ending}").unwrap();
    }

    content
}

fn bench_parse_conflicts(criterion: &mut Criterion) {
    let fixture = large_conflicted_file();
    let mut group = criterion.benchmark_group("git_conflict_parse");
    group.bench_function("600_mixed_merge_and_diff3_regions", |bencher| {
        bencher.iter(|| {
            vcs_git::conflict::parse_conflicts(black_box(&fixture)).expect("fixture is valid")
        })
    });
    group.bench_function("600_regions_parse_and_render_exact", |bencher| {
        bencher.iter(|| {
            let segments =
                vcs_git::conflict::parse_conflicts(black_box(&fixture)).expect("fixture is valid");
            let rendered = vcs_git::conflict::render(black_box(&segments));
            assert_eq!(rendered, fixture, "render must remain byte-exact");
            rendered
        })
    });
    group.finish();
}

criterion_group!(benches, bench_parse_conflicts);
criterion_main!(benches);
