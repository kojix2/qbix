use std::path::{Path, PathBuf};

use crate::commands;
use crate::error::{Error, PublicResult as Result};
use crate::hts::{BamRecord, Header, HtsFile};
use crate::index::{
    generate_index_filename, BamMetadata, Index, DEFAULT_BUCKET_BITS, DEFAULT_INDEX_MEMORY_LIMIT,
};

const BGZF_CACHE_SIZE: usize = 64 * 1024 * 1024;

/// Options used when building a `.qbi` index.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BuildOptions {
    pub index_path: Option<PathBuf>,
    pub threads: usize,
    pub verbose: bool,
    pub memory_limit: Option<usize>,
    pub bucket_bits: Option<u8>,
    pub temp_dir: Option<PathBuf>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            index_path: None,
            threads: 1,
            verbose: false,
            memory_limit: None,
            bucket_bits: None,
            temp_dir: None,
        }
    }
}

/// Options used when opening a BAM and `.qbi` index for lookup.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LookupOptions {
    pub index_path: Option<PathBuf>,
    pub threads: usize,
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            index_path: None,
            threads: 1,
        }
    }
}

/// Options used when checking an index against its BAM file.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CheckOptions {
    pub index_path: Option<PathBuf>,
    pub threads: usize,
    pub verbose: bool,
    pub mode: CheckMode,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            index_path: None,
            threads: 1,
            verbose: false,
            mode: CheckMode::Quick,
        }
    }
}

/// Amount of validation performed by [`check_index`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Quick,
    Full,
}

/// Output ordering for multi-name record retrieval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputOrder {
    Query,
    Bam,
}

/// A BGZF virtual offset inside a BAM file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualOffset(i64);

impl VirtualOffset {
    pub fn as_i64(self) -> i64 {
        self.0
    }
}

/// A verified read-name lookup hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupHit {
    pub read_name: String,
    pub virtual_offset: VirtualOffset,
}

/// A raw row from a `.qbi` index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexRecord {
    pub qhash: u64,
    pub virtual_offset: VirtualOffset,
}

/// Build a `.qbi` index for a BAM file and return the path that was written.
pub fn build_index<P>(input_bam: P, options: BuildOptions) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    validate_threads(options.threads)?;
    let input_bam = path_to_str(input_bam.as_ref(), "BAM path")?;
    let index_path = optional_path_to_str(options.index_path.as_deref(), "index path")?;
    let temp_dir = optional_path_to_str(options.temp_dir.as_deref(), "temporary directory")?;
    commands::build_index(
        input_bam,
        index_path.as_deref(),
        options.verbose,
        options.threads,
        options.memory_limit.unwrap_or(DEFAULT_INDEX_MEMORY_LIMIT),
        options.bucket_bits.unwrap_or(DEFAULT_BUCKET_BITS),
        temp_dir.as_deref(),
    )
    .map_err(Error::from)?;
    let written =
        generate_index_filename(Some(input_bam), index_path.as_deref()).map_err(Error::from)?;
    Ok(PathBuf::from(written))
}

/// Check that a `.qbi` index matches its BAM.
///
/// [`CheckMode::Quick`] checks BAM size, mtime, and header hash. [`CheckMode::Full`]
/// additionally seeks to every indexed record and verifies its read-name hash.
pub fn check_index<P>(input_bam: P, options: CheckOptions) -> Result<()>
where
    P: AsRef<Path>,
{
    validate_threads(options.threads)?;
    let input_bam = path_to_str(input_bam.as_ref(), "BAM path")?;
    let index_path = optional_path_to_str(options.index_path.as_deref(), "index path")?;
    commands::check_index(
        input_bam,
        index_path.as_deref(),
        options.threads,
        options.verbose,
        options.mode.into(),
    )
    .map_err(Error::from)
}

impl From<CheckMode> for commands::CheckMode {
    fn from(mode: CheckMode) -> Self {
        match mode {
            CheckMode::Quick => Self::Quick,
            CheckMode::Full => Self::Full,
        }
    }
}

/// Read raw `(qhash, virtual offset)` rows from a `.qbi` index.
pub fn read_index_records<P>(input_index: P) -> Result<Vec<IndexRecord>>
where
    P: AsRef<Path>,
{
    let input_index = path_to_str(input_index.as_ref(), "index path")?;
    let index = Index::load(None, Some(input_index), None).map_err(Error::from)?;
    let mut records = Vec::with_capacity(index.record_count());
    for idx in 0..index.record_count() {
        let record = index.record(idx).map_err(Error::from)?;
        records.push(IndexRecord {
            qhash: record.qhash,
            virtual_offset: VirtualOffset(record.file_offset),
        });
    }
    Ok(records)
}

/// An opened BAM file with a matching `.qbi` read-name index.
///
/// This handle maintains a seek position in the underlying BAM file and is not
/// intended to be shared across threads.
pub struct IndexedBam {
    bam_path: PathBuf,
    index_path: PathBuf,
    bam: HtsFile,
    header: Header,
    index: Index,
}

impl IndexedBam {
    /// Open a BAM file and load its matching `.qbi` index.
    pub fn open<P>(input_bam: P, options: LookupOptions) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        validate_threads(options.threads)?;
        let input_bam_str = path_to_str(input_bam.as_ref(), "BAM path")?;
        let index_path_str = optional_path_to_str(options.index_path.as_deref(), "index path")?;

        let resolved_index_path =
            generate_index_filename(Some(input_bam_str), index_path_str.as_deref())
                .map_err(Error::from)?;

        let bam = HtsFile::open(input_bam_str, "r")
            .map_err(|_| format!("[qbix] could not open BAM file: {input_bam_str}"))?;
        bam.set_threads(options.threads).map_err(Error::from)?;
        bam.set_bgzf_cache_size(BGZF_CACHE_SIZE)
            .map_err(Error::from)?;
        let header = bam
            .read_header()
            .map_err(|_| format!("[qbix] could not read BAM header: {input_bam_str}"))?;
        let bam_metadata =
            BamMetadata::from_bam(input_bam_str, header.text_hash().map_err(Error::from)?)
                .map_err(Error::from)?;
        let index = Index::load(
            Some(input_bam_str),
            index_path_str.as_deref(),
            Some(bam_metadata),
        )
        .map_err(Error::from)?;

        Ok(Self {
            bam_path: input_bam.as_ref().to_path_buf(),
            index_path: PathBuf::from(resolved_index_path),
            bam,
            header,
            index,
        })
    }

    /// Path of the BAM file passed to [`IndexedBam::open`].
    pub fn bam_path(&self) -> &Path {
        &self.bam_path
    }

    /// Resolved path of the `.qbi` index in use.
    pub fn index_path(&self) -> &Path {
        &self.index_path
    }

    /// Number of rows in the loaded index.
    pub fn record_count(&self) -> usize {
        self.index.record_count()
    }

    /// Return verified offsets for records whose BAM `QNAME` exactly matches `read_name`.
    ///
    /// Hash matches from the index are checked against the BAM record before an
    /// offset is returned, so hash collisions do not produce false hits.
    pub fn lookup_offsets(&self, read_name: &str) -> Result<Vec<VirtualOffset>> {
        self.lookup(read_name).map(|hits| {
            hits.into_iter()
                .map(|hit| hit.virtual_offset)
                .collect::<Vec<_>>()
        })
    }

    /// Return unverified candidate offsets for matching read-name hashes.
    ///
    /// These offsets are read directly from the `.qbi` hash table and may
    /// include false candidates if a 64-bit hash collision occurs. Prefer
    /// [`IndexedBam::lookup_offsets`] unless raw candidates are specifically needed.
    pub fn lookup_offsets_unverified(&self, read_name: &str) -> Result<Vec<VirtualOffset>> {
        let mut offsets = Vec::new();
        for idx in self.index.range_indices(read_name).map_err(Error::from)? {
            let record = self.index.record(idx).map_err(Error::from)?;
            offsets.push(VirtualOffset(record.file_offset));
        }
        Ok(offsets)
    }

    /// Return verified lookup hits for records whose BAM `QNAME` exactly matches `read_name`.
    pub fn lookup(&self, read_name: &str) -> Result<Vec<LookupHit>> {
        let rec = BamRecord::new().map_err(Error::from)?;
        let mut hits = Vec::new();
        for offset in self.lookup_offsets_unverified(read_name)? {
            self.bam
                .read_record_at(&self.header, &rec, offset.as_i64())
                .map_err(Error::from)?;
            if rec.qname().map_err(Error::from)? == read_name {
                hits.push(LookupHit {
                    read_name: read_name.to_string(),
                    virtual_offset: offset,
                });
            }
        }
        Ok(hits)
    }

    /// Write verified matching records as SAM to `output_path`.
    pub fn write_sam_records_to_path<P, S>(
        &self,
        output_path: P,
        read_names: &[S],
        order: OutputOrder,
    ) -> Result<usize>
    where
        P: AsRef<Path>,
        S: AsRef<str>,
    {
        let output_path = path_to_str(output_path.as_ref(), "output path")?;
        let out = HtsFile::open(output_path, "w")
            .map_err(|_| format!("[qbix] could not open SAM output: {output_path}"))?;
        let rec = BamRecord::new().map_err(Error::from)?;
        let mut hits = Vec::new();

        for read_name in read_names {
            let read_name = read_name.as_ref();
            for idx in self.index.range_indices(read_name).map_err(Error::from)? {
                let record = self.index.record(idx).map_err(Error::from)?;
                hits.push((read_name, record.file_offset));
            }
        }
        if order == OutputOrder::Bam {
            hits.sort_by_key(|(_, file_offset)| *file_offset);
        }

        let mut written = 0usize;
        for (read_name, file_offset) in hits {
            self.bam
                .read_record_at(&self.header, &rec, file_offset)
                .map_err(Error::from)?;
            if rec.qname().map_err(Error::from)? == read_name {
                out.write_record(&self.header, &rec).map_err(Error::from)?;
                written += 1;
            }
        }
        Ok(written)
    }
}

fn validate_threads(threads: usize) -> Result<()> {
    if threads == 0 {
        return Err(Error::new("[qbix] threads must be a positive integer"));
    }
    Ok(())
}

fn optional_path_to_str(path: Option<&Path>, what: &str) -> Result<Option<String>> {
    path.map(|path| path_to_str(path, what).map(str::to_string))
        .transpose()
}

fn path_to_str<'a>(path: &'a Path, what: &str) -> Result<&'a str> {
    path.to_str()
        .ok_or_else(|| Error::new(format!("[qbix] {what} is not valid UTF-8")))
}
