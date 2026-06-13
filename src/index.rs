use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::UNIX_EPOCH;

use crate::error::Result;
use memmap2::Mmap;
use xxhash_rust::xxh3::xxh3_64;

const INDEX_IO_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const MAGIC: &[u8; 4] = b"QBI1";
const HEADER_SIZE: u16 = 48;
const RECORD_SIZE: u16 = 16;
const HEADER_SIZE_OFFSET: usize = 4;
const RECORD_SIZE_OFFSET: usize = 6;
const NAME_BYTES_OFFSET: usize = 8;
const RECORD_COUNT_OFFSET: usize = 16;
const BAM_SIZE_OFFSET: usize = 24;
const BAM_MTIME_OFFSET: usize = 32;
const BAM_HEADER_HASH_OFFSET: usize = 40;
const RECORD_QHASH_OFFSET: usize = 0;
const RECORD_FILE_OFFSET: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BamMetadata {
    size: u64,
    mtime: u64,
    header_hash: u64,
}

impl BamMetadata {
    pub(crate) fn from_bam(input_bam: &str, header_hash: u64) -> Result<Self> {
        let metadata = std::fs::metadata(input_bam)
            .map_err(|e| format!("[qbix] could not stat BAM file '{input_bam}': {e}"))?;
        let mtime = metadata
            .modified()
            .map_err(|e| format!("[qbix] could not read BAM mtime '{input_bam}': {e}"))?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| format!("[qbix] BAM mtime is before Unix epoch: {input_bam}"))?;
        let mtime = u64::try_from(mtime.as_nanos())
            .map_err(|_| format!("[qbix] BAM mtime is too large: {input_bam}"))?;

        Ok(Self {
            size: metadata.len(),
            mtime,
            header_hash,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Record {
    pub(crate) qhash: u64,
    pub(crate) file_offset: i64,
}

#[derive(Debug)]
enum IndexStorage {
    Owned { records: Vec<Record> },
    Mapped(MappedIndex),
}

#[derive(Debug)]
struct MappedIndex {
    mmap: Mmap,
    record_start: usize,
    record_count: usize,
}

#[derive(Clone, Copy)]
struct RawDiskRecord {
    qhash: u64,
    file_offset: i64,
}

struct OwnedIndexMut<'a> {
    records: &'a mut Vec<Record>,
}

impl IndexStorage {
    fn owned_mut(&mut self) -> Result<OwnedIndexMut<'_>> {
        match self {
            Self::Owned { records } => Ok(OwnedIndexMut { records }),
            Self::Mapped(_) => Err("[qbix] cannot modify a memory-mapped index".to_string()),
        }
    }
}

impl MappedIndex {
    fn raw_record(&self, index: usize) -> Result<RawDiskRecord> {
        if index >= self.record_count {
            return Err("[qbix] corrupt index: record offset is out of range".to_string());
        }
        let offset = self.record_start + index * usize::from(RECORD_SIZE);
        let qhash = read_u64_le_from(
            &self.mmap[offset + RECORD_QHASH_OFFSET..offset + RECORD_FILE_OFFSET],
            "index records",
        )?;
        let file_offset = read_u64_le_i64_from(
            &self.mmap[offset + RECORD_FILE_OFFSET..offset + 16],
            "index records",
        )?;
        Ok(RawDiskRecord { qhash, file_offset })
    }

    fn record(&self, index: usize) -> Result<Record> {
        let raw = self.raw_record(index)?;
        Ok(Record {
            qhash: raw.qhash,
            file_offset: raw.file_offset,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Index {
    storage: IndexStorage,
}

impl Index {
    pub(crate) fn new() -> Self {
        Self {
            storage: IndexStorage::Owned {
                records: Vec::new(),
            },
        }
    }

    pub(crate) fn add(&mut self, readname: &str, file_offset: i64) -> Result<()> {
        if file_offset < 0 {
            return Err("[qbix] cannot index a negative BGZF offset".to_string());
        }
        let owned = self.storage.owned_mut()?;
        owned.records.push(Record {
            qhash: qname_hash64(readname.as_bytes()),
            file_offset,
        });
        Ok(())
    }

    pub(crate) fn save(&mut self, filename: &str, bam_metadata: BamMetadata) -> Result<()> {
        let owned = self.storage.owned_mut()?;
        owned.records.sort_by(|a, b| {
            a.qhash
                .cmp(&b.qhash)
                .then_with(|| a.file_offset.cmp(&b.file_offset))
        });

        let file = File::create(filename)
            .map_err(|e| format!("[qbix] could not open index for writing '{filename}': {e}"))?;
        let mut fp = BufWriter::with_capacity(INDEX_IO_BUFFER_SIZE, file);
        fp.write_all(MAGIC)
            .map_err(|_| "[qbix] write error while writing file magic".to_string())?;
        write_u16_le(&mut fp, HEADER_SIZE, "header size")?;
        write_u16_le(&mut fp, RECORD_SIZE, "record size")?;
        write_u64_le(&mut fp, 0usize, "read name byte count")?;
        write_u64_le(&mut fp, owned.records.len(), "record count")?;
        write_u64_le(&mut fp, bam_metadata.size, "BAM size")?;
        write_u64_le(&mut fp, bam_metadata.mtime, "BAM mtime")?;
        write_u64_le(&mut fp, bam_metadata.header_hash, "BAM header hash")?;

        for record in owned.records.iter() {
            write_u64_le(&mut fp, record.qhash, "index record")?;
            write_u64_le(&mut fp, record.file_offset, "index record")?;
        }
        fp.flush()
            .map_err(|e| format!("[qbix] could not close index after writing '{filename}': {e}"))?;
        Ok(())
    }

    pub(crate) fn load(
        input_bam: Option<&str>,
        input_index: Option<&str>,
        expected_bam_metadata: Option<BamMetadata>,
    ) -> Result<Self> {
        let index_fn = generate_index_filename(input_bam, input_index)?;
        let file = File::open(&index_fn)
            .map_err(|_| format!("[qbix] index file not found: {index_fn}"))?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| format!("[qbix] could not mmap index '{index_fn}': {e}"))?;
        if mmap.len() < usize::from(HEADER_SIZE) {
            return Err("[qbix] corrupt index: file is shorter than header".to_string());
        }
        if &mmap[..4] != MAGIC {
            return Err("[qbix] unsupported index format: expected QBI1".to_string());
        }

        Self::load_mapped(mmap, expected_bam_metadata)
    }

    fn load_mapped(mmap: Mmap, expected_bam_metadata: Option<BamMetadata>) -> Result<Self> {
        let header_size =
            read_u16_le_from(&mmap[HEADER_SIZE_OFFSET..RECORD_SIZE_OFFSET], "header size")?;
        if header_size != HEADER_SIZE {
            return Err(format!(
                "[qbix] unsupported index header size: {header_size}"
            ));
        }
        let record_size =
            read_u16_le_from(&mmap[RECORD_SIZE_OFFSET..NAME_BYTES_OFFSET], "record size")?;
        if record_size != RECORD_SIZE {
            return Err(format!(
                "[qbix] unsupported index record size: {record_size}"
            ));
        }
        let name_count_bytes = read_u64_le_usize_from(
            &mmap[NAME_BYTES_OFFSET..RECORD_COUNT_OFFSET],
            "read name byte count",
        )?;
        let record_count =
            read_u64_le_usize_from(&mmap[RECORD_COUNT_OFFSET..BAM_SIZE_OFFSET], "record count")?;
        let bam_metadata = BamMetadata {
            size: read_u64_le_from(&mmap[BAM_SIZE_OFFSET..BAM_MTIME_OFFSET], "BAM size")?,
            mtime: read_u64_le_from(&mmap[BAM_MTIME_OFFSET..BAM_HEADER_HASH_OFFSET], "BAM mtime")?,
            header_hash: read_u64_le_from(
                &mmap[BAM_HEADER_HASH_OFFSET..usize::from(HEADER_SIZE)],
                "BAM header hash",
            )?,
        };
        if let Some(expected) = expected_bam_metadata {
            validate_bam_metadata(bam_metadata, expected)?;
        }
        if name_count_bytes != 0 {
            return Err("[qbix] incompatible index, please rebuild".to_string());
        }

        let record_start = usize::from(header_size);
        let record_bytes = record_count
            .checked_mul(usize::from(record_size))
            .ok_or_else(|| "[qbix] corrupt index: record table is too large".to_string())?;
        let expected_len = record_start
            .checked_add(record_bytes)
            .ok_or_else(|| "[qbix] corrupt index: record table is too large".to_string())?;
        if mmap.len() != expected_len {
            return Err("[qbix] corrupt index: file size does not match header".to_string());
        }

        Ok(Self {
            storage: IndexStorage::Mapped(MappedIndex {
                mmap,
                record_start,
                record_count,
            }),
        })
    }

    pub(crate) fn record_count(&self) -> usize {
        match &self.storage {
            IndexStorage::Owned { records, .. } => records.len(),
            IndexStorage::Mapped(mapped) => mapped.record_count,
        }
    }

    pub(crate) fn last_record(&self) -> Result<Option<Record>> {
        match &self.storage {
            IndexStorage::Owned { records } => Ok(records.last().copied()),
            IndexStorage::Mapped(mapped) => {
                if mapped.record_count == 0 {
                    Ok(None)
                } else {
                    mapped.record(mapped.record_count - 1).map(Some)
                }
            }
        }
    }

    pub(crate) fn record(&self, index: usize) -> Result<Record> {
        match &self.storage {
            IndexStorage::Owned { records } => records
                .get(index)
                .copied()
                .ok_or_else(|| "[qbix] corrupt index: record offset is out of range".to_string()),
            IndexStorage::Mapped(mapped) => mapped.record(index),
        }
    }

    pub(crate) fn range_indices(&self, readname: &str) -> Result<std::ops::Range<usize>> {
        let qhash = qname_hash64(readname.as_bytes());
        let mut lo = 0usize;
        let mut hi = self.record_count();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.record(mid)?.qhash.cmp(&qhash) {
                Ordering::Less => lo = mid + 1,
                Ordering::Equal | Ordering::Greater => hi = mid,
            }
        }
        let start = lo;

        hi = self.record_count();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.record(mid)?.qhash.cmp(&qhash) {
                Ordering::Greater => hi = mid,
                Ordering::Less | Ordering::Equal => lo = mid + 1,
            }
        }
        Ok(start..lo)
    }
}

pub(crate) fn generate_index_filename(
    input_bam: Option<&str>,
    input_index: Option<&str>,
) -> Result<String> {
    if let Some(input_index) = input_index {
        return Ok(input_index.to_string());
    }
    input_bam
        .map(|input_bam| format!("{input_bam}.qbi"))
        .ok_or_else(|| "[qbix] no BAM filename or index filename was provided".to_string())
}

fn validate_bam_metadata(actual: BamMetadata, expected: BamMetadata) -> Result<()> {
    if actual.size != expected.size {
        return Err("[qbix] index does not match BAM file: size differs".to_string());
    }
    if actual.mtime != expected.mtime {
        return Err("[qbix] index does not match BAM file: mtime differs".to_string());
    }
    if actual.header_hash != expected.header_hash {
        return Err("[qbix] index does not match BAM file: header hash differs".to_string());
    }
    Ok(())
}

pub(crate) fn qname_hash64(qname: &[u8]) -> u64 {
    xxh3_64(qname)
}

fn read_u16_le_from(bytes: &[u8], what: &str) -> Result<u16> {
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| format!("[qbix] read error while reading {what}"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn write_u16_le<W: Write>(writer: &mut W, value: u16, what: &str) -> Result<()> {
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|_| format!("[qbix] write error while writing {what}"))
}

fn read_u64_le_from(bytes: &[u8], what: &str) -> Result<u64> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| format!("[qbix] read error while reading {what}"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_u64_le_usize_from(bytes: &[u8], what: &str) -> Result<usize> {
    let value = read_u64_le_from(bytes, what)?;
    usize::try_from(value).map_err(|_| format!("[qbix] {what} does not fit on this platform"))
}

fn read_u64_le_i64_from(bytes: &[u8], what: &str) -> Result<i64> {
    let value = read_u64_le_from(bytes, what)?;
    i64::try_from(value).map_err(|_| format!("[qbix] {what} is too large for htslib"))
}

fn write_u64_le<W, V>(writer: &mut W, value: V, what: &str) -> Result<()>
where
    W: Write,
    V: TryInto<u64>,
{
    let value = value
        .try_into()
        .map_err(|_| format!("[qbix] {what} cannot be represented as u64"))?;
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|_| format!("[qbix] write error while writing {what}"))
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::{env, process};

    use super::*;

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
        assert_eq!(&magic, MAGIC);
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

        let mut offsets = Vec::new();
        for idx in loaded.range_indices("read_a").unwrap() {
            offsets.push(loaded.record(idx).unwrap().file_offset);
        }
        assert_eq!(offsets, [10, 20]);
        assert!(loaded.range_indices("missing").unwrap().is_empty());
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
        fp.write_all(MAGIC).unwrap();

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
            .write_all(MAGIC)
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

    fn temp_index_path(name: &str) -> std::path::PathBuf {
        env::temp_dir().join(format!("qbix-test-{name}-{}.qbi", process::id()))
    }
}
