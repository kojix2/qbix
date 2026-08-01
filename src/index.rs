use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
#[cfg(test)]
use std::io::BufWriter;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::Result;
use memmap2::Mmap;
use xxhash_rust::xxh3::xxh3_64;

mod qbi2;

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;

use qbi2::{MappedQbi2, Qbi2GroupIter, Qbi2Writer, QBI2_MAGIC};

#[cfg(test)]
#[path = "index_bench.rs"]
mod benchmarks;

const INDEX_IO_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const BUCKET_STAGING_BUFFER_SIZE: usize = 64 * 1024;
const QBI1_MAGIC: &[u8; 4] = b"QBI1";
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
const SECTION_IO_BUFFER_SIZE: usize = 64 * 1024;
pub(crate) const DEFAULT_INDEX_MEMORY_LIMIT: usize = 512 * 1024 * 1024;
pub(crate) const DEFAULT_BUCKET_BITS: u8 = 8;
pub(crate) const DEFAULT_SORT_THREADS: usize = 1;
pub(crate) const MIN_BUCKET_BITS: u8 = 1;
pub(crate) const MAX_BUCKET_BITS: u8 = 12;
// P=16 spends 522,240 extra directory bytes and saves exactly one byte per
// distinct hash, so K=522,240 is the exact size crossover against P=8.
pub(crate) const QBI2_RADIX_SIZE_CROSSOVER: usize = 522_240;

pub(crate) fn estimated_qbi1_size(record_count: usize) -> Result<u64> {
    u64::from(HEADER_SIZE)
        .checked_add(
            u64::try_from(record_count)
                .map_err(|_| "[qbix] record count is too large".to_string())?
                .checked_mul(u64::from(RECORD_SIZE))
                .ok_or_else(|| "[qbix] estimated QBI1 size overflow".to_string())?,
        )
        .ok_or_else(|| "[qbix] estimated QBI1 size overflow".to_string())
}

pub(crate) fn estimated_qbi2_size(
    record_count: usize,
    unique_hash_count: usize,
    radix_bits: u8,
) -> Result<u64> {
    qbi2::estimated_size(record_count, unique_hash_count, radix_bits)
}

pub(crate) fn resolve_qbi2_radix_bits(requested: Option<u8>, record_count: usize) -> u8 {
    // K is unavailable until sorted output has already started. Since K <= N,
    // small N guarantees P=8 is smaller; larger indexes keep the faster P=16.
    requested.unwrap_or({
        if record_count <= QBI2_RADIX_SIZE_CROSSOVER {
            8
        } else {
            16
        }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexFormat {
    #[default]
    Qbi1,
    Qbi2,
}

impl IndexFormat {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Qbi1 => "QBI1",
            Self::Qbi2 => "QBI2",
        }
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
enum IndexStorage {
    #[cfg(test)]
    Owned {
        records: Vec<Record>,
    },
    Mapped(MappedIndex),
}

#[derive(Debug)]
enum MappedIndex {
    Qbi1(MappedQbi1),
    Qbi2(MappedQbi2),
}

#[derive(Debug)]
struct MappedQbi1 {
    mmap: Mmap,
    record_start: usize,
    record_count: usize,
    bam_metadata: BamMetadata,
}

#[cfg(test)]
impl IndexStorage {
    fn owned_mut(&mut self) -> Result<&mut Vec<Record>> {
        match self {
            Self::Owned { records } => Ok(records),
            Self::Mapped(_) => Err("[qbix] cannot modify a memory-mapped index".to_string()),
        }
    }
}

impl MappedQbi1 {
    fn record(&self, index: usize) -> Result<Record> {
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
        Ok(Record { qhash, file_offset })
    }
}

#[derive(Debug)]
pub(crate) struct Index {
    storage: IndexStorage,
}

impl Index {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            storage: IndexStorage::Owned {
                records: Vec::new(),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn add(&mut self, readname: &str, file_offset: i64) -> Result<()> {
        if file_offset < 0 {
            return Err("[qbix] cannot index a negative BGZF offset".to_string());
        }
        let owned = self.storage.owned_mut()?;
        owned.push(Record {
            qhash: qname_hash64(readname.as_bytes()),
            file_offset,
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn save(&mut self, filename: &str, bam_metadata: BamMetadata) -> Result<()> {
        let owned = self.storage.owned_mut()?;
        owned.sort_unstable_by(Record::cmp_key);

        let file = File::create(filename)
            .map_err(|e| format!("[qbix] could not open index for writing '{filename}': {e}"))?;
        let mut fp = BufWriter::with_capacity(INDEX_IO_BUFFER_SIZE, file);
        write_header(&mut fp, owned.len(), bam_metadata)?;

        for record in owned.iter() {
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
        if mmap.len() < 4 {
            return Err("[qbix] corrupt index: file is shorter than header".to_string());
        }
        if &mmap[..4] == QBI1_MAGIC {
            Self::load_qbi1(mmap, expected_bam_metadata)
        } else if &mmap[..4] == QBI2_MAGIC {
            Self::load_qbi2(mmap, expected_bam_metadata)
        } else {
            Err("[qbix] unsupported index format: expected QBI1 or QBI2".to_string())
        }
    }

    fn load_qbi1(mmap: Mmap, expected_bam_metadata: Option<BamMetadata>) -> Result<Self> {
        if mmap.len() < usize::from(HEADER_SIZE) {
            return Err("[qbix] corrupt index: file is shorter than header (QBI1)".to_string());
        }
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
            storage: IndexStorage::Mapped(MappedIndex::Qbi1(MappedQbi1 {
                mmap,
                record_start,
                record_count,
                bam_metadata,
            })),
        })
    }

    fn load_qbi2(mmap: Mmap, expected_bam_metadata: Option<BamMetadata>) -> Result<Self> {
        let mapped = MappedQbi2::load(mmap, expected_bam_metadata)?;
        Ok(Self {
            storage: IndexStorage::Mapped(MappedIndex::Qbi2(mapped)),
        })
    }

    pub(crate) fn format(&self) -> IndexFormat {
        match &self.storage {
            #[cfg(test)]
            IndexStorage::Owned { .. } => IndexFormat::Qbi1,
            IndexStorage::Mapped(MappedIndex::Qbi1(_)) => IndexFormat::Qbi1,
            IndexStorage::Mapped(MappedIndex::Qbi2(_)) => IndexFormat::Qbi2,
        }
    }

    pub(crate) fn qbi2_radix_bits(&self) -> Option<u8> {
        match &self.storage {
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => Some(mapped.radix_bits()),
            _ => None,
        }
    }

    pub(crate) fn validate_full_structure(&self) -> Result<()> {
        match &self.storage {
            #[cfg(test)]
            IndexStorage::Owned { .. } => Ok(()),
            IndexStorage::Mapped(MappedIndex::Qbi1(_)) => Ok(()),
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => mapped.validate_full(),
        }
    }

    pub(crate) fn record_count(&self) -> usize {
        match &self.storage {
            #[cfg(test)]
            IndexStorage::Owned { records, .. } => records.len(),
            IndexStorage::Mapped(MappedIndex::Qbi1(mapped)) => mapped.record_count,
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => mapped.record_count,
        }
    }

    pub(crate) fn bam_metadata(&self) -> Option<BamMetadata> {
        match &self.storage {
            #[cfg(test)]
            IndexStorage::Owned { .. } => None,
            IndexStorage::Mapped(MappedIndex::Qbi1(mapped)) => Some(mapped.bam_metadata),
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => Some(mapped.bam_metadata),
        }
    }

    pub(crate) fn record(&self, index: usize) -> Result<Record> {
        match &self.storage {
            #[cfg(test)]
            IndexStorage::Owned { records } => records
                .get(index)
                .copied()
                .ok_or_else(|| "[qbix] corrupt index: record offset is out of range".to_string()),
            IndexStorage::Mapped(MappedIndex::Qbi1(mapped)) => mapped.record(index),
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => mapped.record(index),
        }
    }

    fn range_for_hash(&self, qhash: u64) -> Result<std::ops::Range<usize>> {
        if let IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) = &self.storage {
            return mapped.candidate_range(qhash);
        }
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

    pub(crate) fn candidate_offsets<'a>(
        &'a self,
        readname: &str,
    ) -> Result<CandidateOffsetIter<'a>> {
        self.candidate_offsets_for_hash(qname_hash64(readname.as_bytes()))
    }

    fn candidate_offsets_for_hash(&self, qhash: u64) -> Result<CandidateOffsetIter<'_>> {
        let range = self.range_for_hash(qhash)?;
        Ok(CandidateOffsetIter {
            index: self,
            next: range.start,
            end: range.end,
        })
    }

    pub(crate) fn iter_records(&self) -> IndexRecordIter<'_> {
        let qbi2_groups = match &self.storage {
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => Some(mapped.iter_groups()),
            _ => None,
        };
        IndexRecordIter {
            index: self,
            position: 0,
            group_end: 0,
            qhash: 0,
            qbi2_groups,
            failed: false,
        }
    }

    pub(crate) fn iter_hash_groups(&self) -> HashGroupIter<'_> {
        let qbi2_groups = match &self.storage {
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => Some(mapped.iter_groups()),
            _ => None,
        };
        HashGroupIter {
            index: self,
            position: 0,
            qbi2_groups,
        }
    }
}

pub(crate) struct IndexRecordIter<'a> {
    index: &'a Index,
    position: usize,
    group_end: usize,
    qhash: u64,
    qbi2_groups: Option<Qbi2GroupIter<'a>>,
    failed: bool,
}

impl Iterator for IndexRecordIter<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.position >= self.index.record_count() {
            return None;
        }
        if let Some(groups) = &mut self.qbi2_groups {
            if self.position == self.group_end {
                let group = match groups.next() {
                    Some(Ok(group)) => group,
                    Some(Err(error)) => {
                        self.failed = true;
                        return Some(Err(error));
                    }
                    None => {
                        self.failed = true;
                        return Some(Err(
                            "[qbix] corrupt QBI2 index: hash group is missing".to_string()
                        ));
                    }
                };
                if group.start != self.position {
                    self.failed = true;
                    return Some(Err(
                        "[qbix] corrupt QBI2 index: noncontiguous hash group".to_string()
                    ));
                }
                self.group_end = group.end;
                self.qhash = group.qhash;
            }
            let IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) = &self.index.storage else {
                unreachable!("QBI2 group iterator requires QBI2 storage");
            };
            let file_offset = match mapped.offset(self.position) {
                Ok(value) => value,
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            };
            self.position += 1;
            return Some(Ok(Record {
                qhash: self.qhash,
                file_offset,
            }));
        }
        let result = self.index.record(self.position);
        self.position += 1;
        Some(result)
    }
}

pub(crate) struct HashGroupIter<'a> {
    index: &'a Index,
    position: usize,
    qbi2_groups: Option<Qbi2GroupIter<'a>>,
}

impl Iterator for HashGroupIter<'_> {
    type Item = Result<(u64, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(groups) = &mut self.qbi2_groups {
            return groups
                .next()
                .map(|result| result.map(|group| (group.qhash, group.end - group.start)));
        }
        if self.position >= self.index.record_count() {
            return None;
        }
        Some((|| {
            let qhash = self.index.record(self.position)?.qhash;
            let start = self.position;
            self.position += 1;
            while self.position < self.index.record_count()
                && self.index.record(self.position)?.qhash == qhash
            {
                self.position += 1;
            }
            Ok((qhash, self.position - start))
        })())
    }
}

pub(crate) struct CandidateOffsetIter<'a> {
    index: &'a Index,
    next: usize,
    end: usize,
}

impl Iterator for CandidateOffsetIter<'_> {
    type Item = Result<i64>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let index = self.next;
        self.next += 1;
        Some(match &self.index.storage {
            #[cfg(test)]
            IndexStorage::Owned { records } => records
                .get(index)
                .map(|record| record.file_offset)
                .ok_or_else(|| "[qbix] corrupt index: record offset is out of range".to_string()),
            IndexStorage::Mapped(MappedIndex::Qbi1(mapped)) => {
                mapped.record(index).map(|record| record.file_offset)
            }
            IndexStorage::Mapped(MappedIndex::Qbi2(mapped)) => mapped.offset(index),
        })
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
    format: IndexFormat,
    qbi2_radix_bits: Option<u8>,
}

impl BucketIndexBuilder {
    #[cfg(test)]
    pub(crate) fn new(
        output_index: &str,
        memory_limit: usize,
        bucket_bits: u8,
        sort_threads: usize,
        temp_dir: Option<&str>,
    ) -> Result<Self> {
        Self::new_with_format(
            output_index,
            memory_limit,
            bucket_bits,
            sort_threads,
            temp_dir,
            IndexFormat::Qbi1,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_format(
        output_index: &str,
        memory_limit: usize,
        bucket_bits: u8,
        sort_threads: usize,
        temp_dir: Option<&str>,
        format: IndexFormat,
    ) -> Result<Self> {
        Self::new_with_format_and_radix(
            output_index,
            memory_limit,
            bucket_bits,
            sort_threads,
            temp_dir,
            format,
            None,
        )
    }

    pub(crate) fn new_with_format_and_radix(
        output_index: &str,
        memory_limit: usize,
        bucket_bits: u8,
        sort_threads: usize,
        temp_dir: Option<&str>,
        format: IndexFormat,
        qbi2_radix_bits: Option<u8>,
    ) -> Result<Self> {
        if memory_limit < usize::from(RECORD_SIZE) {
            return Err("[qbix] memory limit must be at least 16 bytes".to_string());
        }
        if sort_threads == 0 {
            return Err("[qbix] sort threads must be a positive integer".to_string());
        }
        validate_bucket_bits(bucket_bits)?;
        if format == IndexFormat::Qbi2
            && qbi2_radix_bits.is_some_and(|bits| !matches!(bits, 8 | 12 | 16))
        {
            return Err("[qbix] QBI2 radix bits must be 8, 12, or 16".to_string());
        }

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
            format,
            qbi2_radix_bits,
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

        let mut file = File::create(&self.final_tmp_path).map_err(|e| {
            format!(
                "[qbix] could not open temporary index for writing '{}': {e}",
                self.final_tmp_path.display()
            )
        })?;
        let qbi2_radix_bits = resolve_qbi2_radix_bits(self.qbi2_radix_bits, self.total_records);
        let mut sink = SortedRecordSink::new(
            self.format,
            qbi2_radix_bits,
            &mut file,
            self.total_records,
            bam_metadata,
        )?;

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
                    sink.push(&mut file, record)?;
                }
                if bucket.bytes > 0 {
                    let _ = std::fs::remove_file(&bucket.path);
                }
            }
        }

        sink.finish(&mut file).map_err(|e| {
            format!(
                "[qbix] could not close temporary index '{}': {e}",
                self.final_tmp_path.display()
            )
        })?;
        drop(file);
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

enum SortedRecordSink {
    Qbi1(Qbi1Writer),
    Qbi2(Box<Qbi2Writer>),
}

impl SortedRecordSink {
    fn new(
        format: IndexFormat,
        qbi2_radix_bits: u8,
        file: &mut File,
        record_count: usize,
        bam_metadata: BamMetadata,
    ) -> Result<Self> {
        match format {
            IndexFormat::Qbi1 => {
                file.seek(SeekFrom::Start(0))
                    .map_err(|e| format!("[qbix] could not seek temporary index: {e}"))?;
                write_header(file, record_count, bam_metadata)?;
                Ok(Self::Qbi1(Qbi1Writer {
                    records: SectionWriter::with_capacity(
                        usize::from(HEADER_SIZE),
                        INDEX_IO_BUFFER_SIZE,
                    ),
                }))
            }
            IndexFormat::Qbi2 => Ok(Self::Qbi2(Box::new(Qbi2Writer::new(
                record_count,
                bam_metadata,
                qbi2_radix_bits,
            )?))),
        }
    }

    fn push(&mut self, file: &mut File, record: Record) -> Result<()> {
        match self {
            Self::Qbi1(writer) => writer.push(file, record),
            Self::Qbi2(writer) => writer.push(file, record),
        }
    }

    fn finish(self, file: &mut File) -> Result<()> {
        match self {
            Self::Qbi1(writer) => writer.finish(file),
            Self::Qbi2(writer) => writer.finish(file),
        }
    }
}

struct Qbi1Writer {
    records: SectionWriter,
}

impl Qbi1Writer {
    fn push(&mut self, file: &mut File, record: Record) -> Result<()> {
        self.records.write(file, &record.qhash.to_le_bytes())?;
        self.records.write(file, &record.file_offset.to_le_bytes())
    }

    fn finish(mut self, file: &mut File) -> Result<()> {
        self.records.flush(file)?;
        file.flush()
            .map_err(|e| format!("[qbix] could not flush QBI1 index: {e}"))
    }
}

struct SectionWriter {
    start: usize,
    written: usize,
    flush_threshold: usize,
    buffer: Vec<u8>,
}

impl SectionWriter {
    fn new(start: usize) -> Self {
        Self::with_capacity(start, SECTION_IO_BUFFER_SIZE)
    }

    fn with_capacity(start: usize, flush_threshold: usize) -> Self {
        Self {
            start,
            written: 0,
            flush_threshold,
            buffer: Vec::with_capacity(flush_threshold),
        }
    }

    fn write(&mut self, file: &mut File, bytes: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= self.flush_threshold {
            self.flush(file)?;
        }
        Ok(())
    }

    fn flush(&mut self, file: &mut File) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let offset = self
            .start
            .checked_add(self.written)
            .ok_or_else(|| "[qbix] index section offset overflow".to_string())?;
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|e| format!("[qbix] could not seek index section: {e}"))?;
        file.write_all(&self.buffer)
            .map_err(|e| format!("[qbix] could not write index section: {e}"))?;
        self.written += self.buffer.len();
        self.buffer.clear();
        Ok(())
    }
}

fn write_header<W: Write>(
    writer: &mut W,
    record_count: usize,
    bam_metadata: BamMetadata,
) -> Result<()> {
    writer
        .write_all(QBI1_MAGIC)
        .map_err(|_| "[qbix] write error while writing file magic".to_string())?;
    write_u16_le(writer, HEADER_SIZE, "header size")?;
    write_u16_le(writer, RECORD_SIZE, "record size")?;
    write_u64_le(writer, 0usize, "read name byte count")?;
    write_u64_le(writer, record_count, "record count")?;
    write_u64_le(writer, bam_metadata.size, "BAM size")?;
    write_u64_le(writer, bam_metadata.mtime, "BAM mtime")?;
    write_u64_le(writer, bam_metadata.header_hash, "BAM header hash")
}

#[cfg(test)]
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
