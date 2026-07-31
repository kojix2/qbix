use std::io::Read;
use std::{env, process};

use super::*;

#[test]
fn qbi2_auto_radix_uses_only_guaranteed_size_boundary() {
    assert_eq!(resolve_qbi2_radix_bits(None, 0), 8);
    assert_eq!(resolve_qbi2_radix_bits(None, QBI2_RADIX_SIZE_CROSSOVER), 8);
    assert_eq!(
        resolve_qbi2_radix_bits(None, QBI2_RADIX_SIZE_CROSSOVER + 1),
        16
    );
    assert_eq!(resolve_qbi2_radix_bits(Some(8), usize::MAX), 8);
    assert_eq!(resolve_qbi2_radix_bits(Some(16), 0), 16);
}

#[test]
fn save_loads_hash_records_and_preserves_offsets() {
    let mut index = Index::new();
    index.add("read_b", 30).unwrap();
    index.add("read_a", 10).unwrap();
    index.add("read_a", 20).unwrap();

    let path = env::temp_dir().join(format!("qbix-test-{}.qbi", process::id()));
    index
        .save(path.to_str().unwrap(), test_bam_metadata())
        .unwrap();
    let mut magic = [0u8; 4];
    File::open(&path).unwrap().read_exact(&mut magic).unwrap();
    assert_eq!(&magic, QBI1_MAGIC);
    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let _ = std::fs::remove_file(&path);

    assert_eq!(loaded.record_count(), 3);
    let mut got = Vec::new();
    for idx in 0..loaded.record_count() {
        let record = loaded.record(idx).unwrap();
        got.push((record.qhash, record.file_offset));
    }
    let mut expected = vec![
        (qname_hash64(b"read_b"), 30),
        (qname_hash64(b"read_a"), 10),
        (qname_hash64(b"read_a"), 20),
    ];
    expected.sort();
    assert_eq!(got, expected);

    let offsets = loaded
        .candidate_offsets("read_a")
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(offsets, [10, 20]);
    assert!(loaded
        .candidate_offsets("missing")
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn bucket_builder_writes_same_bytes_as_in_memory_save() {
    let records = [
        ("read_b", 30),
        ("read_a", 10),
        ("read_c", 40),
        ("read_a", 20),
    ];
    assert_bucket_builder_matches_in_memory_save(&records, 2, DEFAULT_SORT_THREADS);
}

#[test]
fn bucket_builder_parallel_sort_writes_same_bytes_as_in_memory_save() {
    let records = [
        ("read_b", 30),
        ("read_a", 10),
        ("read_c", 40),
        ("read_a", 20),
        ("read_d", 50),
        ("read_e", 60),
    ];
    assert_bucket_builder_matches_in_memory_save(&records, 3, 3);
}

#[test]
fn bucket_builder_matches_in_memory_save_at_bucket_bit_bounds() {
    let records = [
        ("read_b", 30),
        ("read_a", 10),
        ("read_c", 40),
        ("read_a", 20),
        ("read_d", 50),
        ("read_e", 60),
    ];
    assert_bucket_builder_matches_in_memory_save(&records, MIN_BUCKET_BITS, 2);
    assert_bucket_builder_matches_in_memory_save(&records, MAX_BUCKET_BITS, 2);
}

#[test]
fn qbi2_round_trips_groups_and_bitvector_boundaries() {
    let path = temp_index_path("qbi2-round-trip");
    let mut input = Vec::new();
    for index in 0..520 {
        let name = if index < 70 {
            "large_group".to_string()
        } else {
            format!("read_{index:04}")
        };
        input.push((name, (index + 1) as i64 * 10));
    }
    let mut builder = BucketIndexBuilder::new_with_format(
        path.to_str().unwrap(),
        64 * 1024,
        8,
        2,
        None,
        IndexFormat::Qbi2,
    )
    .unwrap();
    for (name, offset) in &input {
        builder.add(name, *offset).unwrap();
    }
    builder.finish(test_bam_metadata()).unwrap();

    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    assert_eq!(loaded.format(), IndexFormat::Qbi2);
    assert_eq!(loaded.record_count(), input.len());
    let offsets = loaded
        .candidate_offsets("large_group")
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(offsets.len(), 70);
    assert_eq!(offsets[0], 10);
    assert_eq!(offsets[69], 700);
    assert!(loaded
        .candidate_offsets("not_present")
        .unwrap()
        .next()
        .is_none());

    let mut expected = input
        .iter()
        .map(|(name, offset)| (qname_hash64(name.as_bytes()), *offset))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let got = (0..loaded.record_count())
        .map(|index| {
            let record = loaded.record(index).unwrap();
            (record.qhash, record.file_offset)
        })
        .collect::<Vec<_>>();
    assert_eq!(got, expected);
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_round_trips_empty_index() {
    let path = temp_index_path("qbi2-empty");
    let builder = BucketIndexBuilder::new_with_format(
        path.to_str().unwrap(),
        16,
        1,
        1,
        None,
        IndexFormat::Qbi2,
    )
    .unwrap();
    builder.finish(test_bam_metadata()).unwrap();
    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    assert_eq!(loaded.record_count(), 0);
    assert_eq!(loaded.format(), IndexFormat::Qbi2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_rejects_corrupt_rank_and_padding() {
    let path = temp_index_path("qbi2-corrupt-rank");
    build_test_qbi2(&path, &[("read_a", 10), ("read_b", 20)]);
    let bytes = std::fs::read(&path).unwrap();
    let rank_offset = read_u64_le_usize_from(&bytes[80..88], "rank offset").unwrap();
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(rank_offset as u64)).unwrap();
    file.write_all(&1u64.to_le_bytes()).unwrap();
    drop(file);
    let error = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    assert!(error.contains("rank"));

    build_test_qbi2(&path, &[("read_a", 10), ("read_b", 20)]);
    let bytes = std::fs::read(&path).unwrap();
    let group_offset = read_u64_le_usize_from(&bytes[72..80], "group offset").unwrap();
    let mut word = read_u64_le_from(&bytes[group_offset..group_offset + 8], "bits").unwrap();
    word |= 1u64 << 63;
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(group_offset as u64)).unwrap();
    file.write_all(&word.to_le_bytes()).unwrap();
    drop(file);
    let error = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    assert!(error.contains("padding"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_defers_full_rank_validation_until_requested() {
    let path = temp_index_path("qbi2-deferred-rank");
    let records = (0..600u64)
        .map(|qhash| Record {
            qhash,
            file_offset: qhash as i64 + 1,
        })
        .collect::<Vec<_>>();
    write_qbi2_records(&path, &records, 16);

    let bytes = std::fs::read(&path).unwrap();
    let rank_offset = read_u64_le_usize_from(&bytes[80..88], "rank offset").unwrap();
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start((rank_offset + 8) as u64))
        .unwrap();
    file.write_all(&999u64.to_le_bytes()).unwrap();
    drop(file);

    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let error = loaded.validate_full_structure().unwrap_err();
    assert!(error.contains("rank directory"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_defers_full_suffix_validation_until_requested() {
    let path = temp_index_path("qbi2-deferred-suffix");
    let records = [
        Record {
            qhash: 1,
            file_offset: 10,
        },
        Record {
            qhash: 2,
            file_offset: 20,
        },
    ];
    write_qbi2_records(&path, &records, 16);

    let bytes = std::fs::read(&path).unwrap();
    let suffix_offset = read_u64_le_usize_from(&bytes[96..104], "suffix offset").unwrap();
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(suffix_offset as u64)).unwrap();
    file.write_all(&2u64.to_le_bytes()[..6]).unwrap();
    file.write_all(&1u64.to_le_bytes()[..6]).unwrap();
    drop(file);

    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let error = loaded.validate_full_structure().unwrap_err();
    assert!(error.contains("suffixes are not strictly sorted"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_p8_round_trips() {
    let path = temp_index_path("qbi2-p8");
    let mut records = [
        Record {
            qhash: qname_hash64(b"read_b"),
            file_offset: 30,
        },
        Record {
            qhash: qname_hash64(b"read_a"),
            file_offset: 10,
        },
        Record {
            qhash: qname_hash64(b"read_a"),
            file_offset: 20,
        },
    ];
    records.sort_unstable_by(Record::cmp_key);
    write_qbi2_records(&path, &records, 8);
    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let offsets = loaded
        .candidate_offsets("read_a")
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(offsets, [10, 20]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_select_matches_random_group_boundaries() {
    let path = temp_index_path("qbi2-random-select");
    let mut seed = 0x9e37_79b9u64;
    let mut boundaries = vec![0usize];
    let mut records = Vec::new();
    for group in 0..200u64 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let length = (seed as usize % 17) + 1;
        for offset in 0..length {
            records.push(Record {
                qhash: group << 48,
                file_offset: (records.len() + offset + 1) as i64,
            });
        }
        boundaries.push(records.len());
    }
    let mut file = File::create(&path).unwrap();
    let mut writer = Qbi2Writer::new(records.len(), test_bam_metadata(), 16).unwrap();
    for &record in &records {
        writer.push(&mut file, record).unwrap();
    }
    writer.finish(&mut file).unwrap();
    drop(file);
    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) = &loaded.storage else {
        panic!("expected QBI2");
    };
    for (selected, &expected) in boundaries.iter().enumerate() {
        assert_eq!(mapped.select1(selected).unwrap(), expected);
        if let Some(&end) = boundaries.get(selected + 1) {
            assert_eq!(mapped.select1_pair(selected).unwrap(), (expected, end));
        }
    }

    let groups = loaded
        .iter_hash_groups()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(groups.len(), 200);
    for (group, ((qhash, count), bounds)) in groups.iter().zip(boundaries.windows(2)).enumerate() {
        assert_eq!(*qhash, (group as u64) << 48);
        assert_eq!(*count, bounds[1] - bounds[0]);
    }
    assert_eq!(
        loaded.iter_records().collect::<Result<Vec<_>>>().unwrap(),
        records
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn qbi2_iterators_handle_giant_group_and_sparse_radix() {
    let path = temp_index_path("qbi2-linear-iter-boundaries");
    let mut records = Vec::new();
    for offset in 0..700 {
        records.push(Record {
            qhash: 0,
            file_offset: offset + 1,
        });
    }
    for qhash in 1..=1_025u64 {
        records.push(Record {
            qhash,
            file_offset: records.len() as i64 + 1,
        });
    }
    records.push(Record {
        qhash: u64::MAX,
        file_offset: records.len() as i64 + 1,
    });
    write_qbi2_records(&path, &records, 16);

    let loaded = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let groups = loaded
        .iter_hash_groups()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(groups.first(), Some(&(0, 700)));
    assert_eq!(groups.last(), Some(&(u64::MAX, 1)));
    assert_eq!(groups.len(), 1_027);
    assert_eq!(
        loaded.iter_records().collect::<Result<Vec<_>>>().unwrap(),
        records
    );

    let IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) = &loaded.storage else {
        panic!("expected QBI2");
    };
    assert_eq!(mapped.select1_pair(0).unwrap(), (0, 700));
    assert_eq!(mapped.select1_pair(1_026).unwrap(), (1_725, 1_726));
    let _ = std::fs::remove_file(path);
}

fn build_test_qbi2(path: &Path, records: &[(&str, i64)]) {
    let mut builder = BucketIndexBuilder::new_with_format(
        path.to_str().unwrap(),
        1024,
        2,
        1,
        None,
        IndexFormat::Qbi2,
    )
    .unwrap();
    for (name, offset) in records {
        builder.add(name, *offset).unwrap();
    }
    builder.finish(test_bam_metadata()).unwrap();
}

fn write_qbi2_records(path: &Path, records: &[Record], radix_bits: u8) {
    let mut file = File::create(path).unwrap();
    let mut writer = Qbi2Writer::new(records.len(), test_bam_metadata(), radix_bits).unwrap();
    for record in records {
        writer.push(&mut file, *record).unwrap();
    }
    writer.finish(&mut file).unwrap();
}

#[test]
fn bucket_builder_rejects_oversized_bucket() {
    let path = temp_index_path("oversized-bucket");
    let mut builder = BucketIndexBuilder::new(path.to_str().unwrap(), 16, 1, 1, None).unwrap();
    builder.add("same-read", 10).unwrap();

    let err = builder.add("same-read", 20).unwrap_err();
    assert!(err.contains("bucket"));
    assert!(err.contains("too large"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn bucket_builder_cleans_temp_dir_after_flushed_oversized_bucket_error() {
    let temp_parent = env::temp_dir().join(format!("qbix-flushed-error-cleanup-{}", process::id()));
    std::fs::create_dir_all(&temp_parent).unwrap();
    let path = temp_index_path("flushed-oversized-bucket");
    {
        let mut builder = BucketIndexBuilder::new(
            path.to_str().unwrap(),
            BUCKET_STAGING_BUFFER_SIZE,
            MIN_BUCKET_BITS,
            DEFAULT_SORT_THREADS,
            Some(temp_parent.to_str().unwrap()),
        )
        .unwrap();
        for offset in 0..(BUCKET_STAGING_BUFFER_SIZE / RECORD_SIZE_BYTES) {
            builder.add("same-read", offset as i64).unwrap();
        }
        assert!(temp_parent
            .read_dir()
            .unwrap()
            .next()
            .expect("work directory should exist after flush")
            .unwrap()
            .path()
            .read_dir()
            .unwrap()
            .next()
            .is_some());

        let err = builder
            .add(
                "same-read",
                (BUCKET_STAGING_BUFFER_SIZE / RECORD_SIZE_BYTES) as i64,
            )
            .unwrap_err();
        assert!(err.contains("too large"));
    }

    assert!(temp_parent.read_dir().unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(&temp_parent);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_rejects_mismatched_bam_metadata() {
    let mut index = Index::new();
    index.add("read_a", 10).unwrap();

    let path = temp_index_path("metadata-mismatch");
    index
        .save(path.to_str().unwrap(), test_bam_metadata())
        .unwrap();

    let expected = BamMetadata {
        size: 999,
        ..test_bam_metadata()
    };
    let err = Index::load(None, Some(path.to_str().unwrap()), Some(expected)).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("index does not match BAM file"));
}

#[test]
fn load_rejects_legacy_v1_indexes() {
    let path = temp_index_path("legacy-v1");
    let mut fp = File::create(&path).unwrap();
    fp.write_all(&1usize.to_ne_bytes()).unwrap();
    fp.write_all(&[0u8; 48]).unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("unsupported index format"));
}

#[test]
fn load_rejects_short_headers() {
    let path = temp_index_path("short-header");
    let mut fp = File::create(&path).unwrap();
    fp.write_all(QBI1_MAGIC).unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("shorter than header"));
}

#[test]
fn load_rejects_unsupported_header_size() {
    let path = temp_index_path("bad-header-size");
    let mut fp = File::create(&path).unwrap();
    write_header_custom(&mut fp, HEADER_SIZE - 1, RECORD_SIZE, 0, 0).unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("unsupported index header size"));
}

#[test]
fn load_rejects_unsupported_record_size() {
    let path = temp_index_path("bad-record-size");
    let mut fp = File::create(&path).unwrap();
    write_header_custom(&mut fp, HEADER_SIZE, RECORD_SIZE + 1, 0, 0).unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("unsupported index record size"));
}

#[test]
fn load_rejects_file_size_mismatch() {
    let path = temp_index_path("size-mismatch");
    let mut fp = File::create(&path).unwrap();
    write_header(&mut fp, 0, 1).unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("file size does not match header"));
}

#[test]
fn load_rejects_incompatible_name_table_indexes() {
    let path = temp_index_path("name-table-index");
    let mut fp = File::create(&path).unwrap();
    write_header(&mut fp, 2, 1).unwrap();
    fp.write_all(b"a\0").unwrap();
    write_u64_le(&mut fp, qname_hash64(b"a"), "record qhash").unwrap();
    write_u64_le(&mut fp, 1i64, "record file offset").unwrap();

    let err = Index::load(None, Some(path.to_str().unwrap()), None).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("incompatible index"));
}

#[test]
fn load_rejects_file_offsets_too_large_for_htslib() {
    let path = temp_index_path("too-large-offset");
    let mut fp = File::create(&path).unwrap();
    write_header(&mut fp, 0, 1).unwrap();
    write_u64_le(&mut fp, qname_hash64(b"a"), "record qhash").unwrap();
    write_u64_le(&mut fp, u64::MAX, "record file offset").unwrap();

    let index = Index::load(None, Some(path.to_str().unwrap()), None).unwrap();
    let err = index.record(0).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("too large for htslib"));
}

fn write_header<W: Write>(
    writer: &mut W,
    name_count_bytes: usize,
    record_count: usize,
) -> Result<()> {
    write_header_custom(
        writer,
        HEADER_SIZE,
        RECORD_SIZE,
        name_count_bytes,
        record_count,
    )
}

fn write_header_custom<W: Write>(
    writer: &mut W,
    header_size: u16,
    record_size: u16,
    name_count_bytes: usize,
    record_count: usize,
) -> Result<()> {
    writer
        .write_all(QBI1_MAGIC)
        .map_err(|_| "[qbix] write error while writing file magic".to_string())?;
    write_u16_le(writer, header_size, "header size")?;
    write_u16_le(writer, record_size, "record size")?;
    write_u64_le(writer, name_count_bytes, "read name byte count")?;
    write_u64_le(writer, record_count, "record count")?;
    let metadata = test_bam_metadata();
    write_u64_le(writer, metadata.size, "BAM size")?;
    write_u64_le(writer, metadata.mtime, "BAM mtime")?;
    write_u64_le(writer, metadata.header_hash, "BAM header hash")
}

fn test_bam_metadata() -> BamMetadata {
    BamMetadata {
        size: 123,
        mtime: 456,
        header_hash: 789,
    }
}

fn assert_bucket_builder_matches_in_memory_save(
    records: &[(&str, i64)],
    bucket_bits: u8,
    sort_threads: usize,
) {
    let metadata = test_bam_metadata();
    let in_memory_path = temp_index_path(&format!(
        "in-memory-bits-{bucket_bits}-threads-{sort_threads}"
    ));
    let bucket_path = temp_index_path(&format!("bucket-bits-{bucket_bits}-threads-{sort_threads}"));
    let bucket_tmp = env::temp_dir().join(format!(
        "qbix-bucket-test-bits-{bucket_bits}-threads-{sort_threads}-{}",
        process::id()
    ));
    std::fs::create_dir_all(&bucket_tmp).unwrap();

    let mut index = Index::new();
    let mut builder = BucketIndexBuilder::new(
        bucket_path.to_str().unwrap(),
        DEFAULT_INDEX_MEMORY_LIMIT,
        bucket_bits,
        sort_threads,
        Some(bucket_tmp.to_str().unwrap()),
    )
    .unwrap();
    for (readname, offset) in records {
        index.add(readname, *offset).unwrap();
        builder.add(readname, *offset).unwrap();
    }

    index
        .save(in_memory_path.to_str().unwrap(), metadata)
        .unwrap();
    builder.finish(metadata).unwrap();

    let in_memory = std::fs::read(&in_memory_path).unwrap();
    let bucket = std::fs::read(&bucket_path).unwrap();
    let _ = std::fs::remove_file(&in_memory_path);
    let _ = std::fs::remove_file(&bucket_path);
    let _ = std::fs::remove_dir_all(&bucket_tmp);

    assert_eq!(bucket, in_memory, "bucket_bits={bucket_bits}");
}

fn temp_index_path(name: &str) -> std::path::PathBuf {
    env::temp_dir().join(format!("qbix-test-{name}-{}.qbi", process::id()))
}
