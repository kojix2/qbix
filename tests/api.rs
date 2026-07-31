mod common;

use std::path::{Path, PathBuf};

use common::{write_unmapped_bam, TempDir};

#[test]
fn public_error_implements_std_error() {
    fn assert_std_error<T: std::error::Error>() {}

    assert_std_error::<qbix::Error>();
}

#[test]
fn public_api_builds_opens_and_queries_an_index() {
    let temp = TempDir::new("api");
    let bam = temp.path().join("reads.bam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_b", "read_a", "read_a"]);

    let index_path = qbix::build_index(&bam, qbix::BuildOptions::default()).unwrap();
    assert_eq!(index_path, PathBuf::from(format!("{bam_str}.qbi")));

    qbix::check_index(&bam, qbix::CheckOptions::default()).unwrap();
    let mut check_options = qbix::CheckOptions::default();
    check_options.mode = qbix::CheckMode::Full;
    qbix::check_index(&bam, check_options).unwrap();

    let records = qbix::read_index_records(&index_path).unwrap();
    assert_eq!(records.len(), 3);
    assert!(records
        .iter()
        .all(|record| record.virtual_offset.as_i64() >= 0));

    let mut indexed = qbix::IndexedBam::open(&bam, qbix::LookupOptions::default()).unwrap();
    assert_eq!(indexed.bam_path(), bam.as_path());
    assert_eq!(indexed.index_path(), index_path.as_path());
    assert_eq!(indexed.record_count(), 3);

    let unverified_offsets = indexed.lookup_offsets_unverified("read_a").unwrap();
    assert_eq!(unverified_offsets.len(), 2);

    let offsets = indexed.lookup_offsets("read_a").unwrap();
    assert_eq!(offsets.len(), 2);
    assert!(offsets.iter().all(|offset| offset.as_i64() >= 0));

    let hits = indexed.lookup("read_a").unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.read_name == "read_a"));
    assert!(indexed.lookup("missing").unwrap().is_empty());
}

#[test]
fn public_api_writes_sam_records_to_a_path() {
    let temp = TempDir::new("api-write");
    let bam = temp.path().join("reads.bam");
    let query_sam = temp.path().join("query.sam");
    let bam_sam = temp.path().join("bam.sam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_b", "read_a", "read_a"]);

    qbix::build_index(&bam, qbix::BuildOptions::default()).unwrap();
    let mut indexed = qbix::IndexedBam::open(&bam, qbix::LookupOptions::default()).unwrap();
    let query_written = indexed
        .write_sam_records_to_path(&query_sam, &["read_a", "read_b"], qbix::OutputOrder::Query)
        .unwrap();
    let bam_written = indexed
        .write_sam_records_to_path(&bam_sam, &["read_a", "read_b"], qbix::OutputOrder::Bam)
        .unwrap();

    assert_eq!(query_written, 3);
    assert_eq!(bam_written, 3);
    assert_eq!(sam_read_names(&query_sam), ["read_a", "read_a", "read_b"]);
    assert_eq!(sam_read_names(&bam_sam), ["read_b", "read_a", "read_a"]);
}

fn sam_read_names(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('@'))
        .map(|line| line.split('\t').next().unwrap().to_string())
        .collect()
}
