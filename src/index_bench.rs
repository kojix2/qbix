use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

const HASH_COUNT: usize = 500_000;
const RECORDS_PER_HASH: usize = 2;
const QUERY_COUNT: usize = 100_000;
const LOOKUP_ROUNDS: usize = 7;
const SCAN_ROUNDS: usize = 3;
const OPEN_ROUNDS: usize = 100;

#[test]
#[ignore = "manual release-mode lookup microbenchmark"]
fn benchmark_qbi_lookup_smoke() {
    let record_count = HASH_COUNT * RECORDS_PER_HASH;
    let qbi1_path = temp_index_path("qbi1");
    let qbi2_p8_path = temp_index_path("qbi2-p8");
    let qbi2_p16_path = temp_index_path("qbi2-p16");
    let qbi1_build = write_index(&qbi1_path, IndexFormat::Qbi1, 16);
    let qbi2_p8_build = write_index(&qbi2_p8_path, IndexFormat::Qbi2, 8);
    let qbi2_p16_build = write_index(&qbi2_p16_path, IndexFormat::Qbi2, 16);
    let (present, absent) = queries();

    eprintln!(
        "format\tbytes\tencode_ms\topen_us\tpresent_ns/query\tabsent_ns/query\trecord_scan_ms\tgroup_scan_ms\thits"
    );
    for (name, path, build_time) in [
        ("QBI1", &qbi1_path, qbi1_build),
        ("QBI2-P8", &qbi2_p8_path, qbi2_p8_build),
        ("QBI2-P16", &qbi2_p16_path, qbi2_p16_build),
    ] {
        let index = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
        assert_eq!(index.record_count(), record_count);
        let (present_time, hits) = benchmark_queries(&index, &present);
        let (absent_time, absent_hits) = benchmark_queries(&index, &absent);
        let record_scan = benchmark_record_scan(&index);
        let group_scan = benchmark_group_scan(&index);
        assert_eq!(hits, QUERY_COUNT * RECORDS_PER_HASH);
        assert_eq!(absent_hits, 0);
        let open_time = benchmark_open(path);
        eprintln!(
            "{name}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{hits}",
            std::fs::metadata(path).unwrap().len(),
            build_time.as_secs_f64() * 1_000.0,
            open_time.as_secs_f64() * 1_000_000.0 / OPEN_ROUNDS as f64,
            present_time.as_secs_f64() * 1_000_000_000.0 / (QUERY_COUNT * LOOKUP_ROUNDS) as f64,
            absent_time.as_secs_f64() * 1_000_000_000.0 / (QUERY_COUNT * LOOKUP_ROUNDS) as f64,
            record_scan.as_secs_f64() * 1_000.0 / SCAN_ROUNDS as f64,
            group_scan.as_secs_f64() * 1_000.0 / SCAN_ROUNDS as f64,
        );
    }

    for path in [qbi1_path, qbi2_p8_path, qbi2_p16_path] {
        let _ = std::fs::remove_file(path);
    }
}

fn benchmark_record_scan(index: &Index) -> Duration {
    let start = Instant::now();
    let mut checksum = 0u64;
    for _ in 0..SCAN_ROUNDS {
        for record in index.iter_records() {
            let record = record.unwrap();
            checksum = checksum.wrapping_add(record.qhash ^ record.file_offset as u64);
        }
    }
    std::hint::black_box(checksum);
    start.elapsed()
}

fn benchmark_group_scan(index: &Index) -> Duration {
    let start = Instant::now();
    let mut checksum = 0u64;
    for _ in 0..SCAN_ROUNDS {
        for group in index.iter_hash_groups() {
            let (qhash, count) = group.unwrap();
            checksum = checksum.wrapping_add(qhash ^ count as u64);
        }
    }
    std::hint::black_box(checksum);
    start.elapsed()
}

fn write_index(path: &Path, format: IndexFormat, radix_bits: u8) -> Duration {
    let record_count = HASH_COUNT * RECORDS_PER_HASH;
    let step = u64::MAX / HASH_COUNT as u64;
    let start = Instant::now();
    let mut file = File::create(path).unwrap();
    let mut sink = SortedRecordSink::new(
        format,
        radix_bits,
        &mut file,
        record_count,
        benchmark_metadata(),
    )
    .unwrap();
    for group in 0..HASH_COUNT {
        let qhash = group as u64 * step;
        for duplicate in 0..RECORDS_PER_HASH {
            sink.push(
                &mut file,
                Record {
                    qhash,
                    file_offset: (group * RECORDS_PER_HASH + duplicate + 1) as i64,
                },
            )
            .unwrap();
        }
    }
    sink.finish(&mut file).unwrap();
    start.elapsed()
}

fn queries() -> (Vec<u64>, Vec<u64>) {
    let step = u64::MAX / HASH_COUNT as u64;
    let mut state = 0x243f_6a88_85a3_08d3u64;
    let mut present = Vec::with_capacity(QUERY_COUNT);
    let mut absent = Vec::with_capacity(QUERY_COUNT);
    for _ in 0..QUERY_COUNT {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let qhash = (state % HASH_COUNT as u64) * step;
        present.push(qhash);
        absent.push(qhash + step / 2);
    }
    (present, absent)
}

fn benchmark_queries(index: &Index, queries: &[u64]) -> (Duration, usize) {
    let mut warmup = 0i64;
    for &qhash in queries.iter().take(1_000) {
        for offset in index
            .candidate_offsets_for_hash(std::hint::black_box(qhash))
            .unwrap()
        {
            warmup = warmup.wrapping_add(offset.unwrap());
        }
    }
    std::hint::black_box(warmup);

    let start = Instant::now();
    let mut hits = 0usize;
    let mut checksum = 0i64;
    for _ in 0..LOOKUP_ROUNDS {
        for &qhash in queries {
            for offset in index
                .candidate_offsets_for_hash(std::hint::black_box(qhash))
                .unwrap()
            {
                checksum = checksum.wrapping_add(offset.unwrap());
                hits += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    std::hint::black_box((hits, checksum));
    (elapsed, hits / LOOKUP_ROUNDS)
}

fn benchmark_open(path: &Path) -> Duration {
    let start = Instant::now();
    for _ in 0..OPEN_ROUNDS {
        let index = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
        std::hint::black_box(index.record_count());
    }
    start.elapsed()
}

fn benchmark_metadata() -> BamMetadata {
    BamMetadata {
        size: 123,
        mtime: 456,
        header_hash: 789,
    }
}

fn temp_index_path(format: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "qbix-lookup-bench-{format}-{}.qbi",
        std::process::id()
    ))
}
