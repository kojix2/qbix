use std::cmp::Ordering;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

use memmap2::Mmap;

use super::{
    read_u16_le_from, read_u64_le_from, read_u64_le_i64_from, read_u64_le_usize_from,
    validate_bam_metadata, write_u16_le, BamMetadata, Record, SectionWriter,
};
use crate::error::Result;

pub(super) const QBI2_MAGIC: &[u8; 4] = b"QBI2";
const QBI2_HEADER_SIZE: usize = 128;
const QBI2_RANK_BLOCK_WORDS_LOG2: u8 = 3;
const QBI2_HASH_ALGORITHM_XXH3_64: u8 = 1;

struct Qbi2Layout {
    group_word_count: usize,
    rank_entry_count: usize,
    radix_entry_count: usize,
    offsets_offset: usize,
    group_bits_offset: usize,
    rank_offset: usize,
    radix_offset: usize,
    suffix_offset: usize,
}

impl Qbi2Layout {
    fn new(record_count: usize, radix_bits: u8) -> Result<Self> {
        if !matches!(radix_bits, 8 | 12 | 16) {
            return Err(format!("[qbix] unsupported QBI2 radix bits: {radix_bits}"));
        }
        let group_word_count = record_count
            .checked_add(1)
            .and_then(|bits| bits.checked_add(63))
            .map(|bits| bits / 64)
            .ok_or_else(|| "[qbix] QBI2 bit vector is too large".to_string())?;
        let rank_entry_count = group_word_count
            .div_ceil(8)
            .checked_add(1)
            .ok_or_else(|| "[qbix] QBI2 rank directory is too large".to_string())?;
        let radix_entry_count = (1usize << radix_bits) + 1;
        let offsets_offset = QBI2_HEADER_SIZE;
        let group_bits_offset =
            checked_section_end(offsets_offset, record_count, 8, "offset array")?;
        let rank_offset =
            checked_section_end(group_bits_offset, group_word_count, 8, "group bit vector")?;
        let radix_offset = checked_section_end(rank_offset, rank_entry_count, 8, "rank directory")?;
        let suffix_offset =
            checked_section_end(radix_offset, radix_entry_count, 8, "radix directory")?;
        Ok(Self {
            group_word_count,
            rank_entry_count,
            radix_entry_count,
            offsets_offset,
            group_bits_offset,
            rank_offset,
            radix_offset,
            suffix_offset,
        })
    }

    fn file_size(&self, unique_hash_count: usize, suffix_bytes: usize) -> Result<usize> {
        checked_section_end(
            self.suffix_offset,
            unique_hash_count,
            suffix_bytes,
            "suffix array",
        )
    }
}

pub(super) fn estimated_size(
    record_count: usize,
    unique_hash_count: usize,
    radix_bits: u8,
) -> Result<u64> {
    let suffix_bytes = usize::from((64 - radix_bits).div_ceil(8));
    let size =
        Qbi2Layout::new(record_count, radix_bits)?.file_size(unique_hash_count, suffix_bytes)?;
    u64::try_from(size).map_err(|_| "[qbix] estimated QBI2 size overflow".to_string())
}

#[derive(Debug)]
pub(super) struct MappedQbi2 {
    mmap: Mmap,
    pub(super) record_count: usize,
    pub(super) unique_hash_count: usize,
    pub(super) bam_metadata: BamMetadata,
    radix_bits: u8,
    suffix_bytes: usize,
    offsets_offset: usize,
    group_bits_offset: usize,
    group_word_count: usize,
    rank_offset: usize,
    rank_entry_count: usize,
    radix_offset: usize,
    radix_entry_count: usize,
    suffix_offset: usize,
}

impl MappedQbi2 {
    pub(super) fn radix_bits(&self) -> u8 {
        self.radix_bits
    }

    pub(super) fn load(mmap: Mmap, expected_bam_metadata: Option<BamMetadata>) -> Result<Self> {
        if mmap.len() < QBI2_HEADER_SIZE {
            return Err("[qbix] corrupt index: file is shorter than header (QBI2)".to_string());
        }
        let header_size = read_u16_le_from(&mmap[4..6], "QBI2 header size")?;
        if usize::from(header_size) != QBI2_HEADER_SIZE {
            return Err(format!(
                "[qbix] unsupported QBI2 header size: {header_size}"
            ));
        }
        let flags = read_u16_le_from(&mmap[6..8], "QBI2 flags")?;
        if flags != 0 {
            return Err(format!("[qbix] unsupported QBI2 flags: {flags}"));
        }
        let radix_bits = mmap[8];
        let suffix_bytes = usize::from(mmap[9]);
        if !matches!((radix_bits, suffix_bytes), (8, 7) | (12, 7) | (16, 6)) {
            return Err(format!(
                "[qbix] unsupported QBI2 radix/suffix parameters: {radix_bits}/{suffix_bytes}"
            ));
        }
        if mmap[10] != QBI2_RANK_BLOCK_WORDS_LOG2 {
            return Err(format!(
                "[qbix] unsupported QBI2 rank block size: {}",
                mmap[10]
            ));
        }
        if mmap[11] != QBI2_HASH_ALGORITHM_XXH3_64 {
            return Err(format!(
                "[qbix] unsupported QBI2 hash algorithm: {}",
                mmap[11]
            ));
        }
        if mmap[12..16].iter().any(|byte| *byte != 0) {
            return Err("[qbix] unsupported QBI2 reserved header data".to_string());
        }

        let record_count = read_u64_le_usize_from(&mmap[16..24], "QBI2 record count")?;
        let unique_hash_count = read_u64_le_usize_from(&mmap[24..32], "QBI2 unique hash count")?;
        if unique_hash_count > record_count {
            return Err(
                "[qbix] corrupt QBI2 index: unique hash count exceeds record count".to_string(),
            );
        }
        let bam_metadata = BamMetadata {
            size: read_u64_le_from(&mmap[32..40], "BAM size")?,
            mtime: read_u64_le_from(&mmap[40..48], "BAM mtime")?,
            header_hash: read_u64_le_from(&mmap[48..56], "BAM header hash")?,
        };
        if let Some(expected) = expected_bam_metadata {
            validate_bam_metadata(bam_metadata, expected)?;
        }
        if read_u64_le_from(&mmap[56..64], "QBI2 hash seed")? != 0 {
            return Err("[qbix] unsupported QBI2 hash seed".to_string());
        }

        let offsets_offset = read_u64_le_usize_from(&mmap[64..72], "offsets offset")?;
        let group_bits_offset = read_u64_le_usize_from(&mmap[72..80], "group bits offset")?;
        let rank_offset = read_u64_le_usize_from(&mmap[80..88], "rank offset")?;
        let radix_offset = read_u64_le_usize_from(&mmap[88..96], "radix offset")?;
        let suffix_offset = read_u64_le_usize_from(&mmap[96..104], "suffix offset")?;
        let file_size = read_u64_le_usize_from(&mmap[104..112], "QBI2 file size")?;
        let radix_entry_count = read_u64_le_usize_from(&mmap[112..120], "radix entry count")?;
        let rank_entry_count = read_u64_le_usize_from(&mmap[120..128], "rank entry count")?;

        let layout = Qbi2Layout::new(record_count, radix_bits)?;
        if rank_entry_count != layout.rank_entry_count
            || radix_entry_count != layout.radix_entry_count
        {
            return Err(
                "[qbix] corrupt QBI2 index: directory count does not match parameters".to_string(),
            );
        }

        let expected_file_size = layout.file_size(unique_hash_count, suffix_bytes)?;
        if offsets_offset != layout.offsets_offset
            || group_bits_offset != layout.group_bits_offset
            || rank_offset != layout.rank_offset
            || radix_offset != layout.radix_offset
            || suffix_offset != layout.suffix_offset
            || file_size != expected_file_size
            || mmap.len() != expected_file_size
        {
            return Err(
                "[qbix] corrupt QBI2 index: section layout does not match header".to_string(),
            );
        }

        let mapped = Self {
            mmap,
            record_count,
            unique_hash_count,
            bam_metadata,
            radix_bits,
            suffix_bytes,
            offsets_offset: layout.offsets_offset,
            group_bits_offset: layout.group_bits_offset,
            group_word_count: layout.group_word_count,
            rank_offset: layout.rank_offset,
            rank_entry_count: layout.rank_entry_count,
            radix_offset: layout.radix_offset,
            radix_entry_count: layout.radix_entry_count,
            suffix_offset: layout.suffix_offset,
        };
        mapped.validate_lightweight()?;
        Ok(mapped)
    }

    fn validate_lightweight(&self) -> Result<()> {
        // Opening is on every query path, so keep it constant-work. Monotonic
        // directory and suffix scans belong to explicit `check --full`.
        if self.radix(0)? != 0 || self.radix(self.radix_entry_count - 1)? != self.unique_hash_count
        {
            return Err("[qbix] corrupt QBI2 index: invalid radix endpoints".to_string());
        }
        if self.rank(0)? != 0 || self.rank(self.rank_entry_count - 1)? != self.unique_hash_count + 1
        {
            return Err("[qbix] corrupt QBI2 index: invalid rank endpoints".to_string());
        }
        if !self.bit(0)? || !self.bit(self.record_count)? {
            return Err("[qbix] corrupt QBI2 index: group sentinel is missing".to_string());
        }

        let used_bits = (self.record_count + 1) % 64;
        if used_bits != 0 {
            let padding_mask = !((1u64 << used_bits) - 1);
            if self.group_word(self.group_word_count - 1)? & padding_mask != 0 {
                return Err("[qbix] corrupt QBI2 index: nonzero bit-vector padding".to_string());
            }
        }
        Ok(())
    }

    pub(super) fn validate_full(&self) -> Result<()> {
        self.validate_lightweight()?;
        let mut previous = 0usize;
        for index in 0..self.radix_entry_count {
            let value = self.radix(index)?;
            if value < previous || value > self.unique_hash_count {
                return Err("[qbix] corrupt QBI2 index: invalid radix directory".to_string());
            }
            previous = value;
        }
        let mut ones = 0usize;
        for word_index in 0..self.group_word_count {
            if word_index % 8 == 0 {
                let rank_index = word_index / 8;
                if self.rank(rank_index)? != ones {
                    return Err("[qbix] corrupt QBI2 index: invalid rank directory".to_string());
                }
            }
            ones = ones
                .checked_add(self.group_word(word_index)?.count_ones() as usize)
                .ok_or_else(|| "[qbix] corrupt QBI2 index: rank overflow".to_string())?;
        }
        if self.rank(self.rank_entry_count - 1)? != ones || ones != self.unique_hash_count + 1 {
            return Err("[qbix] corrupt QBI2 index: rank total does not match groups".to_string());
        }
        for prefix in 0..self.radix_entry_count - 1 {
            let start = self.radix(prefix)?;
            let end = self.radix(prefix + 1)?;
            let mut last = None;
            for index in start..end {
                let suffix = self.suffix(index)?;
                let suffix_mask = (1u64 << (64 - self.radix_bits)) - 1;
                if suffix > suffix_mask {
                    return Err(
                        "[qbix] corrupt QBI2 index: suffix has nonzero padding bits".to_string()
                    );
                }
                if last.is_some_and(|value| value >= suffix) {
                    return Err(
                        "[qbix] corrupt QBI2 index: suffixes are not strictly sorted".to_string(),
                    );
                }
                last = Some(suffix);
            }
        }
        Ok(())
    }

    pub(super) fn offset(&self, index: usize) -> Result<i64> {
        if index >= self.record_count {
            return Err("[qbix] corrupt index: record offset is out of range".to_string());
        }
        let start = self.offsets_offset + index * 8;
        read_u64_le_i64_from(&self.mmap[start..start + 8], "QBI2 offset array")
    }

    fn group_word(&self, index: usize) -> Result<u64> {
        if index >= self.group_word_count {
            return Err("[qbix] corrupt QBI2 index: bit-vector word is out of range".to_string());
        }
        let start = self.group_bits_offset + index * 8;
        read_u64_le_from(&self.mmap[start..start + 8], "QBI2 group bit vector")
    }

    fn bit(&self, position: usize) -> Result<bool> {
        Ok(self.group_word(position / 64)? & (1u64 << (position % 64)) != 0)
    }

    fn rank(&self, index: usize) -> Result<usize> {
        if index >= self.rank_entry_count {
            return Err("[qbix] corrupt QBI2 index: rank entry is out of range".to_string());
        }
        let start = self.rank_offset + index * 8;
        read_u64_le_usize_from(&self.mmap[start..start + 8], "QBI2 rank directory")
    }

    fn radix(&self, index: usize) -> Result<usize> {
        if index >= self.radix_entry_count {
            return Err("[qbix] corrupt QBI2 index: radix entry is out of range".to_string());
        }
        let start = self.radix_offset + index * 8;
        read_u64_le_usize_from(&self.mmap[start..start + 8], "QBI2 radix directory")
    }

    fn suffix(&self, index: usize) -> Result<u64> {
        if index >= self.unique_hash_count {
            return Err("[qbix] corrupt QBI2 index: suffix entry is out of range".to_string());
        }
        let start = self.suffix_offset + index * self.suffix_bytes;
        let mut bytes = [0u8; 8];
        bytes[..self.suffix_bytes].copy_from_slice(&self.mmap[start..start + self.suffix_bytes]);
        Ok(u64::from_le_bytes(bytes))
    }

    pub(super) fn select1(&self, selected: usize) -> Result<usize> {
        self.select1_with_block(selected)
            .map(|(position, _)| position)
    }

    fn select1_with_block(&self, selected: usize) -> Result<(usize, usize)> {
        if selected > self.unique_hash_count {
            return Err("[qbix] corrupt QBI2 index: select is out of range".to_string());
        }
        let mut lo = 0usize;
        let mut hi = self.rank_entry_count - 1;
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rank(mid)? <= selected {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let mut seen = self.rank(lo)?;
        let start_word = lo * 8;
        for word_index in start_word..(start_word + 8).min(self.group_word_count) {
            let mut word = self.group_word(word_index)?;
            let count = word.count_ones() as usize;
            let end = seen
                .checked_add(count)
                .ok_or_else(|| "[qbix] corrupt QBI2 index: rank overflow".to_string())?;
            if selected < end {
                for _ in seen..selected {
                    word &= word - 1;
                }
                let position = word_index * 64 + word.trailing_zeros() as usize;
                if position > self.record_count {
                    return Err(
                        "[qbix] corrupt QBI2 index: select result is out of range".to_string()
                    );
                }
                return Ok((position, lo));
            }
            seen = end;
        }
        Err("[qbix] corrupt QBI2 index: select target was not found".to_string())
    }

    pub(super) fn select1_pair(&self, selected: usize) -> Result<(usize, usize)> {
        if selected >= self.unique_hash_count {
            return Err("[qbix] corrupt QBI2 index: hash group is out of range".to_string());
        }
        let (start, block) = self.select1_with_block(selected)?;
        // Most adjacent group starts share a 512-bit rank block. Reuse that
        // lookup, but retain bounded select for groups spanning the block.
        let first_word = start / 64;
        let end_word = ((block + 1) * 8).min(self.group_word_count);
        for word_index in first_word..end_word {
            let mut word = self.group_word(word_index)?;
            if word_index == first_word {
                let first_bit = start % 64 + 1;
                word = if first_bit == 64 {
                    0
                } else {
                    word & (u64::MAX << first_bit)
                };
            }
            if word != 0 {
                let end = word_index * 64 + word.trailing_zeros() as usize;
                if end > self.record_count {
                    return Err(
                        "[qbix] corrupt QBI2 index: select result is out of range".to_string()
                    );
                }
                return Ok((start, end));
            }
        }
        Ok((start, self.select1(selected + 1)?))
    }

    pub(super) fn candidate_range(&self, qhash: u64) -> Result<std::ops::Range<usize>> {
        let prefix = (qhash >> (64 - self.radix_bits)) as usize;
        let suffix_mask = (1u64 << (64 - self.radix_bits)) - 1;
        let wanted = qhash & suffix_mask;
        let mut lo = self.radix(prefix)?;
        let mut hi = self.radix(prefix + 1)?;
        if lo > hi || hi > self.unique_hash_count {
            return Err("[qbix] corrupt QBI2 index: invalid radix range".to_string());
        }
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.suffix(mid)?.cmp(&wanted) {
                Ordering::Less => lo = mid + 1,
                Ordering::Equal => {
                    let (start, end) = self.select1_pair(mid)?;
                    if start >= end || end > self.record_count {
                        return Err("[qbix] corrupt QBI2 index: invalid hash group".to_string());
                    }
                    return Ok(start..end);
                }
                Ordering::Greater => hi = mid,
            }
        }
        Ok(0..0)
    }

    pub(super) fn record(&self, index: usize) -> Result<Record> {
        if index >= self.record_count {
            return Err("[qbix] corrupt index: record offset is out of range".to_string());
        }
        let word_index = index / 64;
        let block = word_index / 8;
        let mut group = self.rank(block)?;
        for current in block * 8..word_index {
            group = group
                .checked_add(self.group_word(current)?.count_ones() as usize)
                .ok_or_else(|| "[qbix] corrupt QBI2 index: rank overflow".to_string())?;
        }
        let mask = if index % 64 == 63 {
            u64::MAX
        } else {
            (1u64 << (index % 64 + 1)) - 1
        };
        group = group
            .checked_add((self.group_word(word_index)? & mask).count_ones() as usize)
            .ok_or_else(|| "[qbix] corrupt QBI2 index: rank overflow".to_string())?;
        let group = group
            .checked_sub(1)
            .ok_or_else(|| "[qbix] corrupt QBI2 index: record has no hash group".to_string())?;

        let qhash = self.qhash_for_group(group)?;
        Ok(Record {
            qhash,
            file_offset: self.offset(index)?,
        })
    }

    pub(super) fn qhash_for_group(&self, group: usize) -> Result<u64> {
        if group >= self.unique_hash_count {
            return Err("[qbix] corrupt QBI2 index: hash group is out of range".to_string());
        }
        let mut lo = 0usize;
        let mut hi = self.radix_entry_count - 1;
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            if self.radix(mid)? <= group {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Ok(((lo as u64) << (64 - self.radix_bits)) | self.suffix(group)?)
    }

    pub(super) fn iter_groups(&self) -> Qbi2GroupIter<'_> {
        Qbi2GroupIter {
            mapped: self,
            group: 0,
            prefix: 0,
            word_index: 0,
            remaining_word: None,
            pending_start: None,
            failed: false,
        }
    }
}

pub(super) struct Qbi2Group {
    pub(super) qhash: u64,
    pub(super) start: usize,
    pub(super) end: usize,
}

// Full scans must not repeat rank-directory searches for every group. This
// cursor consumes each bit-vector word and radix prefix in forward order.
pub(super) struct Qbi2GroupIter<'a> {
    mapped: &'a MappedQbi2,
    group: usize,
    prefix: usize,
    word_index: usize,
    remaining_word: Option<u64>,
    pending_start: Option<usize>,
    failed: bool,
}

impl Qbi2GroupIter<'_> {
    fn next_boundary(&mut self) -> Result<usize> {
        loop {
            if self.remaining_word.is_none() {
                if self.word_index >= self.mapped.group_word_count {
                    return Err("[qbix] corrupt QBI2 index: group boundary is missing".to_string());
                }
                self.remaining_word = Some(self.mapped.group_word(self.word_index)?);
            }
            let word = self.remaining_word.as_mut().expect("group word is loaded");
            if *word == 0 {
                self.word_index += 1;
                self.remaining_word = None;
                continue;
            }
            let position = self.word_index * 64 + word.trailing_zeros() as usize;
            *word &= *word - 1;
            if position > self.mapped.record_count {
                return Err("[qbix] corrupt QBI2 index: group boundary is out of range".to_string());
            }
            return Ok(position);
        }
    }

    fn qhash(&mut self) -> Result<u64> {
        while self.prefix + 1 < self.mapped.radix_entry_count
            && self.mapped.radix(self.prefix + 1)? <= self.group
        {
            self.prefix += 1;
        }
        if self.prefix + 1 >= self.mapped.radix_entry_count
            || self.mapped.radix(self.prefix)? > self.group
            || self.mapped.radix(self.prefix + 1)? <= self.group
        {
            return Err("[qbix] corrupt QBI2 index: invalid radix directory".to_string());
        }
        Ok(((self.prefix as u64) << (64 - self.mapped.radix_bits))
            | self.mapped.suffix(self.group)?)
    }
}

impl Iterator for Qbi2GroupIter<'_> {
    type Item = Result<Qbi2Group>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.group >= self.mapped.unique_hash_count {
            return None;
        }
        let result = (|| {
            let start = match self.pending_start.take() {
                Some(start) => start,
                None => self.next_boundary()?,
            };
            let end = self.next_boundary()?;
            if (self.group == 0 && start != 0)
                || start >= end
                || end > self.mapped.record_count
                || (self.group + 1 == self.mapped.unique_hash_count
                    && end != self.mapped.record_count)
            {
                return Err("[qbix] corrupt QBI2 index: invalid hash group".to_string());
            }
            let qhash = self.qhash()?;
            self.pending_start = Some(end);
            self.group += 1;
            Ok(Qbi2Group { qhash, start, end })
        })();
        if result.is_err() {
            self.failed = true;
        }
        Some(result)
    }
}

fn checked_section_end(start: usize, count: usize, width: usize, what: &str) -> Result<usize> {
    count
        .checked_mul(width)
        .and_then(|bytes| start.checked_add(bytes))
        .ok_or_else(|| format!("[qbix] corrupt QBI2 index: {what} is too large"))
}

pub(super) struct Qbi2Writer {
    record_count: usize,
    bam_metadata: BamMetadata,
    radix_bits: u8,
    suffix_bytes: usize,
    group_word_count: usize,
    rank_entry_count: usize,
    radix_offset: usize,
    radix: Vec<u64>,
    next_radix: usize,
    offsets: SectionWriter,
    group_bits: SectionWriter,
    ranks: SectionWriter,
    suffixes: SectionWriter,
    record_index: usize,
    unique_hash_count: usize,
    previous_hash: Option<u64>,
    current_word_index: usize,
    current_word: u64,
    cumulative_ones: usize,
}

impl Qbi2Writer {
    pub(super) fn new(
        record_count: usize,
        bam_metadata: BamMetadata,
        radix_bits: u8,
    ) -> Result<Self> {
        // K is known only after consuming sorted records. Keep all N-derived
        // sections first and the K-sized suffix array last for one-pass output.
        let layout = Qbi2Layout::new(record_count, radix_bits)?;
        let suffix_bytes = usize::from((64 - radix_bits).div_ceil(8));
        Ok(Self {
            record_count,
            bam_metadata,
            radix_bits,
            suffix_bytes,
            group_word_count: layout.group_word_count,
            rank_entry_count: layout.rank_entry_count,
            radix_offset: layout.radix_offset,
            radix: vec![0; layout.radix_entry_count],
            next_radix: 0,
            offsets: SectionWriter::new(layout.offsets_offset),
            group_bits: SectionWriter::new(layout.group_bits_offset),
            ranks: SectionWriter::new(layout.rank_offset),
            suffixes: SectionWriter::new(layout.suffix_offset),
            record_index: 0,
            unique_hash_count: 0,
            previous_hash: None,
            current_word_index: 0,
            current_word: 0,
            cumulative_ones: 0,
        })
    }

    pub(super) fn push(&mut self, file: &mut File, record: Record) -> Result<()> {
        if self.record_index >= self.record_count {
            return Err("[qbix] internal error: too many sorted records".to_string());
        }
        if let Some(previous) = self.previous_hash {
            if record.qhash < previous {
                return Err("[qbix] internal error: sorted records are out of order".to_string());
            }
        }
        self.offsets
            .write(file, &record.file_offset.to_le_bytes())?;
        if self.previous_hash != Some(record.qhash) {
            self.set_group_start(file, self.record_index)?;
            let prefix = (record.qhash >> (64 - self.radix_bits)) as usize;
            while self.next_radix <= prefix {
                self.radix[self.next_radix] = self.unique_hash_count as u64;
                self.next_radix += 1;
            }
            let mask = (1u64 << (64 - self.radix_bits)) - 1;
            let suffix = (record.qhash & mask).to_le_bytes();
            self.suffixes.write(file, &suffix[..self.suffix_bytes])?;
            self.unique_hash_count += 1;
            self.previous_hash = Some(record.qhash);
        }
        self.record_index += 1;
        Ok(())
    }

    fn set_group_start(&mut self, file: &mut File, position: usize) -> Result<()> {
        let target_word = position / 64;
        while self.current_word_index < target_word {
            self.emit_word(file)?;
        }
        self.current_word |= 1u64 << (position % 64);
        Ok(())
    }

    fn emit_word(&mut self, file: &mut File) -> Result<()> {
        if self.current_word_index.is_multiple_of(8) {
            self.ranks
                .write(file, &(self.cumulative_ones as u64).to_le_bytes())?;
        }
        self.group_bits
            .write(file, &self.current_word.to_le_bytes())?;
        self.cumulative_ones += self.current_word.count_ones() as usize;
        self.current_word = 0;
        self.current_word_index += 1;
        Ok(())
    }

    pub(super) fn finish(mut self, file: &mut File) -> Result<()> {
        if self.record_index != self.record_count {
            return Err("[qbix] internal error: sorted record count changed".to_string());
        }
        self.set_group_start(file, self.record_count)?;
        while self.current_word_index < self.group_word_count {
            self.emit_word(file)?;
        }
        self.ranks
            .write(file, &(self.cumulative_ones as u64).to_le_bytes())?;
        while self.next_radix < self.radix.len() {
            self.radix[self.next_radix] = self.unique_hash_count as u64;
            self.next_radix += 1;
        }

        self.offsets.flush(file)?;
        self.group_bits.flush(file)?;
        self.ranks.flush(file)?;
        // P=16 has 65,537 entries; buffering avoids one syscall per entry.
        let mut radix = SectionWriter::new(self.radix_offset);
        for value in &self.radix {
            radix.write(file, &value.to_le_bytes())?;
        }
        radix.flush(file)?;
        self.suffixes.flush(file)?;
        self.write_header(file)?;
        file.flush()
            .map_err(|e| format!("[qbix] could not flush QBI2 index: {e}"))
    }

    fn write_header(&self, file: &mut File) -> Result<()> {
        let suffix_offset = self.suffixes.start;
        let file_size = checked_section_end(
            suffix_offset,
            self.unique_hash_count,
            self.suffix_bytes,
            "suffix array",
        )?;
        let file_size_u64 = u64::try_from(file_size)
            .map_err(|_| "[qbix] QBI2 file size cannot be represented as u64".to_string())?;
        file.set_len(file_size_u64)
            .map_err(|e| format!("[qbix] could not size QBI2 index: {e}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| format!("[qbix] could not seek QBI2 header: {e}"))?;
        file.write_all(QBI2_MAGIC)
            .map_err(|e| format!("[qbix] could not write QBI2 magic: {e}"))?;
        write_u16_le(file, QBI2_HEADER_SIZE as u16, "QBI2 header size")?;
        write_u16_le(file, 0, "QBI2 flags")?;
        file.write_all(&[
            self.radix_bits,
            self.suffix_bytes as u8,
            QBI2_RANK_BLOCK_WORDS_LOG2,
            QBI2_HASH_ALGORITHM_XXH3_64,
            0,
            0,
            0,
            0,
        ])
        .map_err(|e| format!("[qbix] could not write QBI2 parameters: {e}"))?;
        for value in [
            self.record_count as u64,
            self.unique_hash_count as u64,
            self.bam_metadata.size,
            self.bam_metadata.mtime,
            self.bam_metadata.header_hash,
            0,
            self.offsets.start as u64,
            self.group_bits.start as u64,
            self.ranks.start as u64,
            self.radix_offset as u64,
            suffix_offset as u64,
            file_size as u64,
            self.radix.len() as u64,
            self.rank_entry_count as u64,
        ] {
            file.write_all(&value.to_le_bytes())
                .map_err(|e| format!("[qbix] could not write QBI2 header: {e}"))?;
        }
        Ok(())
    }
}
