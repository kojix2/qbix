use crate::error::Result;
use crate::hts::{BamRecord, HtsFile};
use crate::index::{generate_index_filename, qname_hash64, BamMetadata, Index};

const BGZF_CACHE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GetOrder {
    Query,
    Bam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Sam,
    Bam,
}

impl OutputFormat {
    fn hts_mode(self) -> &'static str {
        match self {
            Self::Sam => "w",
            Self::Bam => "wb",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sam => "SAM",
            Self::Bam => "BAM",
        }
    }
}

struct Hit<'a> {
    readname: &'a str,
    file_offset: i64,
}

pub(crate) fn build_index(
    input_bam: &str,
    output_index: Option<&str>,
    verbose: bool,
    threads: usize,
) -> Result<()> {
    let bam =
        HtsFile::open(input_bam, "r").map_err(|_| format!("[qbix] could not open {input_bam}"))?;
    bam.set_threads(threads)?;
    let header = bam
        .read_header()
        .map_err(|_| format!("[qbix] could not read BAM header from {input_bam}"))?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)?;
    let rec = BamRecord::new()?;
    let mut index = Index::new();
    let mut file_offset = bam
        .tell()
        .map_err(|_| format!("[qbix] {input_bam} is not a BGZF-compressed BAM file"))?;

    loop {
        let ret = bam.read_next(&header, &rec);
        if ret < -1 {
            return Err(format!(
                "[qbix] error while reading BAM records from {input_bam}"
            ));
        }
        if ret < 0 {
            break;
        }

        let readname = rec.qname()?;
        index.add(readname, file_offset)?;
        if verbose && (index.record_count() == 1 || index.record_count().is_multiple_of(100_000)) {
            let last = index.last_record()?.expect("record was just added");
            eprintln!(
                "[qbix] build: record {} [{} {}] {}",
                index.record_count(),
                last.qhash,
                last.file_offset,
                readname
            );
        }
        file_offset = bam.tell()?;
    }

    if verbose {
        eprintln!("[qbix] build: writing to disk...");
    }
    let out_fn = generate_index_filename(Some(input_bam), output_index)?;
    index.save(&out_fn, bam_metadata)?;
    if verbose {
        eprintln!(
            "[qbix] build: wrote index for {} records.",
            index.record_count()
        );
    }
    Ok(())
}

pub(crate) fn get_records(
    input_bam: &str,
    input_index: Option<&str>,
    readnames: &[String],
    threads: usize,
    order: GetOrder,
    output_format: OutputFormat,
    output_path: Option<&str>,
) -> Result<()> {
    let bam = HtsFile::open(input_bam, "r")
        .map_err(|_| format!("[qbix] could not open BAM file: {input_bam}"))?;
    bam.set_threads(threads)?;
    bam.set_bgzf_cache_size(BGZF_CACHE_SIZE)?;
    let header = bam
        .read_header()
        .map_err(|_| format!("[qbix] could not read BAM header: {input_bam}"))?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)?;
    let index = Index::load(Some(input_bam), input_index, Some(bam_metadata))?;
    let output_path = output_path.unwrap_or("-");
    let out = HtsFile::open(output_path, output_format.hts_mode()).map_err(|_| {
        format!(
            "[qbix] could not open {} output: {output_path}",
            output_format.name()
        )
    })?;
    if output_format == OutputFormat::Bam {
        out.set_threads(threads)?;
        out.write_header(&header)?;
    }
    let rec = BamRecord::new()?;

    let mut hits = Vec::new();
    for readname in readnames {
        for idx in index.range_indices(readname)? {
            let record = index.record(idx)?;
            hits.push(Hit {
                readname,
                file_offset: record.file_offset,
            });
        }
    }
    if order == GetOrder::Bam {
        hits.sort_by_key(|hit| hit.file_offset);
    }

    for hit in hits {
        bam.read_record_at(&header, &rec, hit.file_offset)?;
        if rec.qname()? == hit.readname {
            out.write_record(&header, &rec)?;
        }
    }
    Ok(())
}

pub(crate) fn show_index(input_index: &str) -> Result<()> {
    let index = Index::load(None, Some(input_index), None)?;
    for idx in 0..index.record_count() {
        let record = index.record(idx)?;
        println!("{}\t{}", record.qhash, record.file_offset);
    }
    Ok(())
}

pub(crate) fn check_index(
    input_bam: &str,
    input_index: Option<&str>,
    threads: usize,
    verbose: bool,
) -> Result<()> {
    let bam = HtsFile::open(input_bam, "r")
        .map_err(|_| "[qbix] check: could not open BAM file".to_string())?;
    bam.set_threads(threads)
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    bam.set_bgzf_cache_size(BGZF_CACHE_SIZE)
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    let header = bam
        .read_header()
        .map_err(|_| "[qbix] check: could not read BAM header".to_string())?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    let index = Index::load(Some(input_bam), input_index, Some(bam_metadata))
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    let rec =
        BamRecord::new().map_err(|_| "[qbix] check: could not allocate BAM record".to_string())?;

    let mut checked = 0usize;
    for idx in 0..index.record_count() {
        let record = index.record(idx)?;
        bam.read_record_at(&header, &rec, record.file_offset)
            .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
        let got = rec.qname()?;
        let got_hash = qname_hash64(got.as_bytes());
        if verbose {
            eprintln!("[qbix] check: {} {}", record.qhash, got_hash);
        }
        if got_hash != record.qhash {
            return Err(
                "[qbix] check: lookup returned a record with the wrong read name hash".to_string(),
            );
        }
        checked += 1;
        if !verbose && checked.is_multiple_of(1_000_000) {
            eprintln!("[qbix] check: checked {checked} records...");
        }
    }
    Ok(())
}
