use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::Result;
use memmap2::Mmap;
use xxhash_rust::xxh3::xxh3_64;

const INDEX_IO_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const BUCKET_STAGING_BUFFER_SIZE: usize = 64 * 1024;
const MAGIC: &[u8; 4] = b"QBI1";
const HEADER_SIZE: u16 = 48;
const RECORD_SIZE: u16 = 16;
const RECORD_SIZE_BYTES: usize = 16;
const HEADER_SIZE_OFFSET: usize = 4;
const RECORD_SIZE_OFFSET: usize = 6;
const NAME_BYTES_OFFSET: usize = 8;
const RECORD_COUNT_OFFSET: usize = 16;
const BAM_SIZE_OFFSET: usize = 24;
const BAM_MTIME_OFFSET: usize = 32;
const BAM_HEADER_HASH_OFFSET: usize = 40;
const RECORD_QHASH_OFFSET: usize = 0;
const RECORD_FILE_OFFSET: usize = 8;
pub(crate) const DEFAULT_INDEX_MEMORY_LIMIT: usize = 512 * 1024 * 1024;
pub(crate) const DEFAULT_BUCKET_BITS: u8 = 8;
pub(crate) const DEFAULT_SORT_THREADS: usize = 1;
pub(crate) const MIN_BUCKET_BITS: u8 = 1;
pub(crate) const MAX_BUCKET_BITS: u8 = 12;

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

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn mtime(&self) -> u64 {
        self.mtime
    }

    pub(crate) fn header_hash(&self) -> u64 {
        self.header_hash
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Record {
    pub(crate) qhash: u64,
    pub(crate) file_offset: i64,
}

impl Record {
    fn cmp_key(&self, other: &Self) -> Ordering {
        self.qhash
            .cmp(&other.qhash)
            .then_with(|| self.file_offset.cmp(&other.file_offset))
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum IndexStorage {
    Owned { records: Vec<Record> },
    Mapped(MappedIndex),
}

#[derive(Debug)]
struct MappedIndex {
    mmap: Mmap,
    record_start: usize,
    record_count: usize,
    bam_metadata: BamMetadata,
}

#[derive(Clone, Copy)]
struct RawDiskRecord {
    qhash: u64,
    file_offset: i64,
}

#[allow(dead_code)]
struct OwnedIndexMut<'a> {
    records: &'a mut Vec<Record>,
}

#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            storage: IndexStorage::Owned {
                records: Vec::new(),
            },
        }
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn save(&mut self, filename: &str, bam_metadata: BamMetadata) -> Result<()> {
        let owned = self.storage.owned_mut()?;
        owned.records.sort_unstable_by(Record::cmp_key);

        let file = File::create(filename)
            .map_err(|e| format!("[qbix] could not open index for writing '{filename}': {e}"))?;
        let mut fp = BufWriter::with_capacity(INDEX_IO_BUFFER_SIZE, file);
        write_header(&mut fp, owned.records.len(), bam_metadata)?;

        for record in owned.records.iter() {
            write_record(&mut fp, *record)?;
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
                bam_metadata,
            }),
        })
    }

    pub(crate) fn record_count(&self) -> usize {
        match &self.storage {
            IndexStorage::Owned { records, .. } => records.len(),
            IndexStorage::Mapped(mapped) => mapped.record_count,
        }
    }

    pub(crate) fn bam_metadata(&self) -> Option<BamMetadata> {
        match &self.storage {
            IndexStorage::Owned { .. } => None,
            IndexStorage::Mapped(mapped) => Some(mapped.bam_metadata),
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

pub(crate) struct BucketIndexBuilder {
    buckets: Vec<BucketState>,
    bucket_bits: u8,
    memory_limit: usize,
    sort_threads: usize,
    total_records: usize,
    output_path: PathBuf,
    final_tmp_path: PathBuf,
    guard: TempGuard,
}

impl BucketIndexBuilder {
    pub(crate) fn new(
        output_index: &str,
        memory_limit: usize,
        bucket_bits: u8,
        sort_threads: usize,
        temp_dir: Option<&str>,
    ) -> Result<Self> {
        if memory_limit < usize::from(RECORD_SIZE) {
            return Err("[qbix] memory limit must be at least 16 bytes".to_string());
        }
        if sort_threads == 0 {
            return Err("[qbix] sort threads must be a positive integer".to_string());
        }
        validate_bucket_bits(bucket_bits)?;

        let output_path = PathBuf::from(output_index);
        let output_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
        let bucket_parent_dir = temp_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| output_dir.into());
        std::fs::create_dir_all(&bucket_parent_dir).map_err(|e| {
            format!(
                "[qbix] could not create temporary directory '{}': {e}",
                bucket_parent_dir.display()
            )
        })?;
        let bucket_dir = create_unique_work_dir(&bucket_parent_dir)?;
        let final_tmp_path = final_tmp_path(&output_path);
        let mut guard = TempGuard::new();
        guard.track_file(final_tmp_path.clone());
        guard.track_dir(bucket_dir.clone());

        let bucket_count = 1usize << bucket_bits;
        let mut buckets = Vec::with_capacity(bucket_count);
        for bucket in 0..bucket_count {
            let path = bucket_dir.join(format!("bucket-{bucket:04}.tmp"));
            buckets.push(BucketState {
                path,
                buffer: None,
                bytes: 0,
                records: 0,
            });
        }

        Ok(Self {
            buckets,
            bucket_bits,
            memory_limit,
            sort_threads,
            total_records: 0,
            output_path,
            final_tmp_path,
            guard,
        })
    }

    pub(crate) fn add(&mut self, readname: &str, file_offset: i64) -> Result<Record> {
        if file_offset < 0 {
            return Err("[qbix] cannot index a negative BGZF offset".to_string());
        }
        let record = Record {
            qhash: qname_hash64(readname.as_bytes()),
            file_offset,
        };
        let bucket = (record.qhash >> (64 - self.bucket_bits)) as usize;
        let state = &mut self.buckets[bucket];
        state.bytes = state
            .bytes
            .checked_add(u64::from(RECORD_SIZE))
            .ok_or_else(|| "[qbix] bucket is too large".to_string())?;
        if state.bytes
            > u64::try_from(self.memory_limit)
                .map_err(|_| "[qbix] memory limit is too large".to_string())?
        {
            return Err(format!(
                "[qbix] bucket {bucket} is too large; retry with larger --memory or higher --bucket-bits"
            ));
        }
        state.records = state
            .records
            .checked_add(1)
            .ok_or_else(|| "[qbix] too many records for one bucket".to_string())?;
        self.total_records = self
            .total_records
            .checked_add(1)
            .ok_or_else(|| "[qbix] too many records for this platform".to_string())?;
        state
            .push_record(record)
            .map_err(|e| format!("[qbix] could not write bucket temp file: {e}"))?;
        Ok(record)
    }

    pub(crate) fn total_records(&self) -> usize {
        self.total_records
    }

    pub(crate) fn finish(mut self, bam_metadata: BamMetadata) -> Result<()> {
        for bucket in &mut self.buckets {
            bucket
                .flush()
                .map_err(|e| format!("[qbix] could not flush bucket temp file: {e}"))?;
        }

        let file = File::create(&self.final_tmp_path).map_err(|e| {
            format!(
                "[qbix] could not open temporary index for writing '{}': {e}",
                self.final_tmp_path.display()
            )
        })?;
        let mut out = BufWriter::with_capacity(INDEX_IO_BUFFER_SIZE, file);
        write_header(&mut out, self.total_records, bam_metadata)?;

        let sort_threads = self.sort_threads.min(self.buckets.len()).max(1);
        let memory_limit = self.memory_limit;
        for bucket_chunk in self.buckets.chunks(sort_threads) {
            let sorted_buckets = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(bucket_chunk.len());
                for bucket in bucket_chunk {
                    handles.push(scope.spawn(move || bucket.read_sorted_records(memory_limit)));
                }

                let mut sorted_buckets = Vec::with_capacity(handles.len());
                for handle in handles {
                    let records = handle
                        .join()
                        .map_err(|_| "[qbix] bucket sort worker panicked".to_string())??;
                    sorted_buckets.push(records);
                }
                Ok::<_, String>(sorted_buckets)
            })?;

            for (bucket, records) in bucket_chunk.iter().zip(sorted_buckets) {
                for record in records {
                    write_record(&mut out, record)?;
                }
                if bucket.bytes > 0 {
                    let _ = std::fs::remove_file(&bucket.path);
                }
            }
        }

        out.flush().map_err(|e| {
            format!(
                "[qbix] could not close temporary index '{}': {e}",
                self.final_tmp_path.display()
            )
        })?;
        drop(out);
        std::fs::rename(&self.final_tmp_path, &self.output_path).map_err(|e| {
            format!(
                "[qbix] could not rename temporary index '{}' to '{}': {e}",
                self.final_tmp_path.display(),
                self.output_path.display()
            )
        })?;
        self.guard.disarm();
        self.guard.remove_tracked_dirs_best_effort();
        Ok(())
    }
}

struct BucketState {
    path: PathBuf,
    buffer: Option<Vec<u8>>,
    bytes: u64,
    records: u64,
}

impl BucketState {
    fn push_record(&mut self, record: Record) -> std::io::Result<()> {
        let buffer = self
            .buffer
            .get_or_insert_with(|| Vec::with_capacity(BUCKET_STAGING_BUFFER_SIZE));
        buffer.extend_from_slice(&record.qhash.to_le_bytes());
        buffer.extend_from_slice(&record.file_offset.to_le_bytes());
        if buffer.len() >= BUCKET_STAGING_BUFFER_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let Some(buffer) = self.buffer.as_mut() else {
            return Ok(());
        };
        if buffer.is_empty() {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(buffer)?;
        buffer.clear();
        Ok(())
    }

    fn read_sorted_records(&self, memory_limit: usize) -> Result<Vec<Record>> {
        let mut records = self.read_records(memory_limit)?;
        records.sort_unstable_by(Record::cmp_key);
        Ok(records)
    }

    fn read_records(&self, memory_limit: usize) -> Result<Vec<Record>> {
        if self.bytes == 0 {
            return Ok(Vec::new());
        }
        if self.bytes
            > u64::try_from(memory_limit)
                .map_err(|_| "[qbix] memory limit is too large".to_string())?
        {
            return Err(format!(
                "[qbix] bucket '{}' is too large; retry with larger --memory or higher --bucket-bits",
                self.path.display()
            ));
        }
        let capacity = usize::try_from(self.records)
            .map_err(|_| "[qbix] bucket record count does not fit on this platform".to_string())?;
        let expected_bytes = self
            .records
            .checked_mul(u64::from(RECORD_SIZE))
            .ok_or_else(|| "[qbix] bucket size is too large".to_string())?;
        if self.bytes != expected_bytes {
            return Err("[qbix] corrupt bucket temp file: size mismatch".to_string());
        }
        let actual_bytes = std::fs::metadata(&self.path)
            .map_err(|e| {
                format!(
                    "[qbix] could not stat bucket temp file '{}': {e}",
                    self.path.display()
                )
            })?
            .len();
        if actual_bytes != self.bytes {
            return Err("[qbix] corrupt bucket temp file: file size mismatch".to_string());
        }

        let mut file = File::open(&self.path).map_err(|e| {
            format!(
                "[qbix] could not open bucket temp file '{}': {e}",
                self.path.display()
            )
        })?;
        let mut records = Vec::with_capacity(capacity);
        let mut raw = [0u8; RECORD_SIZE_BYTES];
        for _ in 0..self.records {
            file.read_exact(&mut raw).map_err(|e| {
                format!(
                    "[qbix] could not read bucket temp file '{}': {e}",
                    self.path.display()
                )
            })?;
            let qhash = read_u64_le_from(&raw[..8], "bucket record")?;
            let file_offset = read_u64_le_i64_from(&raw[8..], "bucket record")?;
            records.push(Record { qhash, file_offset });
        }
        Ok(records)
    }
}

struct TempGuard {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    armed: bool,
}

impl TempGuard {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            dirs: Vec::new(),
            armed: true,
        }
    }

    fn track_file(&mut self, path: PathBuf) {
        self.files.push(path);
    }

    fn track_dir(&mut self, path: PathBuf) {
        self.dirs.push(path);
    }

    fn remove_tracked_dirs_best_effort(&mut self) {
        for dir in self.dirs.drain(..) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for path in &self.files {
            let _ = std::fs::remove_file(path);
        }
        for path in &self.dirs {
            let _ = std::fs::remove_dir_all(path);
        }
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

fn validate_bucket_bits(bucket_bits: u8) -> Result<()> {
    if !(MIN_BUCKET_BITS..=MAX_BUCKET_BITS).contains(&bucket_bits) {
        return Err(format!(
            "[qbix] bucket bits must be between {MIN_BUCKET_BITS} and {MAX_BUCKET_BITS}"
        ));
    }
    Ok(())
}

fn final_tmp_path(output_path: &Path) -> PathBuf {
    let pid = std::process::id();
    let filename = output_path
        .file_name()
        .map(|name| format!("{}.tmp.{pid}", name.to_string_lossy()))
        .unwrap_or_else(|| format!("qbix-index.tmp.{pid}"));
    output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(filename)
}

fn create_unique_work_dir(parent: &Path) -> Result<PathBuf> {
    const MAX_TRIES: usize = 100;
    let pid = std::process::id();
    for attempt in 0..MAX_TRIES {
        let unique = temp_unique_suffix();
        let path = parent.join(format!("qbix-buckets-{pid}-{attempt:03}-{unique}.tmp"));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "[qbix] could not create temporary directory '{}': {e}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "[qbix] could not create a unique temporary directory in '{}'",
        parent.display()
    ))
}

fn temp_unique_suffix() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{now}", std::process::id())
}

fn write_header<W: Write>(
    writer: &mut W,
    record_count: usize,
    bam_metadata: BamMetadata,
) -> Result<()> {
    writer
        .write_all(MAGIC)
        .map_err(|_| "[qbix] write error while writing file magic".to_string())?;
    write_u16_le(writer, HEADER_SIZE, "header size")?;
    write_u16_le(writer, RECORD_SIZE, "record size")?;
    write_u64_le(writer, 0usize, "read name byte count")?;
    write_u64_le(writer, record_count, "record count")?;
    write_u64_le(writer, bam_metadata.size, "BAM size")?;
    write_u64_le(writer, bam_metadata.mtime, "BAM mtime")?;
    write_u64_le(writer, bam_metadata.header_hash, "BAM header hash")
}

fn write_record<W: Write>(writer: &mut W, record: Record) -> Result<()> {
    write_u64_le(writer, record.qhash, "index record")?;
    write_u64_le(writer, record.file_offset, "index record")
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
        let temp_parent =
            env::temp_dir().join(format!("qbix-flushed-error-cleanup-{}", process::id()));
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

    fn assert_bucket_builder_matches_in_memory_save(
        records: &[(&str, i64)],
        bucket_bits: u8,
        sort_threads: usize,
    ) {
        let metadata = test_bam_metadata();
        let in_memory_path = temp_index_path(&format!(
            "in-memory-bits-{bucket_bits}-threads-{sort_threads}"
        ));
        let bucket_path =
            temp_index_path(&format!("bucket-bits-{bucket_bits}-threads-{sort_threads}"));
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
}
