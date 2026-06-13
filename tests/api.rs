mod common;

use std::path::PathBuf;

use common::{write_unmapped_bam, TempDir};

#[test]
fn public_error_works_with_boxed_std_error() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::new("api-boxed-error");
    let bam = temp.path().join("reads.bam");
    write_unmapped_bam(bam.to_str().unwrap(), &["read_a"]);

    qbix::build_index(&bam, qbix::BuildOptions::default())?;
    let indexed = qbix::IndexedBam::open(&bam, qbix::LookupOptions::default())?;
    let _hits = indexed.lookup("read_a")?;
    Ok(())
}

#[test]
fn public_api_builds_opens_and_queries_an_index() {
    let temp = TempDir::new("api");
    let bam = temp.path().join("reads.bam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_b", "read_a", "read_a"]);

    let index_path = qbix::build_index(&bam, qbix::BuildOptions::default()).unwrap();
    assert_eq!(index_path, PathBuf::from(format!("{bam_str}.qbi")));

    qbix::validate_index(&bam, qbix::ValidateOptions::default()).unwrap();

    let records = qbix::read_index_records(&index_path).unwrap();
    assert_eq!(records.len(), 3);
    assert!(records
        .iter()
        .all(|record| record.virtual_offset.as_i64() >= 0));

    let indexed = qbix::IndexedBam::open(&bam, qbix::LookupOptions::default()).unwrap();
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
    let sam = temp.path().join("hits.sam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_b", "read_a", "read_a"]);

    qbix::build_index(&bam, qbix::BuildOptions::default()).unwrap();
    let indexed = qbix::IndexedBam::open(&bam, qbix::LookupOptions::default()).unwrap();
    let written = indexed
        .write_sam_records_to_path(&sam, &["read_a", "read_b"], qbix::OutputOrder::Bam)
        .unwrap();

    assert_eq!(written, 3);
    let sam = std::fs::read_to_string(sam).unwrap();
    let read_names: Vec<_> = sam
        .lines()
        .filter(|line| !line.starts_with('@'))
        .map(|line| line.split('\t').next().unwrap())
        .collect();
    assert_eq!(read_names, ["read_b", "read_a", "read_a"]);
}
