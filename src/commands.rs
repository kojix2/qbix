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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckMode {
    Quick,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatsFormat {
    Text,
    Json,
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
    mode: CheckMode,
) -> Result<()> {
    let bam = HtsFile::open(input_bam, "r")
        .map_err(|_| "[qbix] check: could not open BAM file".to_string())?;
    bam.set_threads(threads)
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    let header = bam
        .read_header()
        .map_err(|_| "[qbix] check: could not read BAM header".to_string())?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    let index = Index::load(Some(input_bam), input_index, Some(bam_metadata))
        .map_err(|e| e.replace("[qbix]", "[qbix] check:"))?;
    if mode == CheckMode::Quick {
        eprintln!("[qbix] check: ok (quick, {} records)", index.record_count());
        return Ok(());
    }

    bam.set_bgzf_cache_size(BGZF_CACHE_SIZE)
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
    eprintln!("[qbix] check: ok (full, {checked} records)");
    Ok(())
}

pub(crate) fn stats_index(
    input_bam: &str,
    input_index: Option<&str>,
    format: StatsFormat,
) -> Result<()> {
    let index_path = generate_index_filename(Some(input_bam), input_index)?;
    let index_size = std::fs::metadata(&index_path)
        .map_err(|e| format!("[qbix] could not stat index file '{index_path}': {e}"))?
        .len();
    let index = Index::load(Some(input_bam), input_index, None)?;
    let stats = compute_qname_hash_stats(&index)?;

    match format {
        StatsFormat::Text => print_stats_text(input_bam, &index_path, index_size, &index, &stats),
        StatsFormat::Json => print_stats_json(input_bam, &index_path, index_size, &index, &stats),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct QnameHashStats {
    records: usize,
    distinct_hashes: usize,
    singleton_hashes: usize,
    pair_hashes: usize,
    multi_hashes: usize,
    max_records_per_hash: usize,
    mean_records_per_hash: f64,
}

fn compute_qname_hash_stats(index: &Index) -> Result<QnameHashStats> {
    let records = index.record_count();
    let mut distinct_hashes = 0usize;
    let mut singleton_hashes = 0usize;
    let mut pair_hashes = 0usize;
    let mut multi_hashes = 0usize;
    let mut max_records_per_hash = 0usize;

    let mut idx = 0usize;
    while idx < records {
        let qhash = index.record(idx)?.qhash;
        let mut run_len = 1usize;
        idx += 1;
        while idx < records && index.record(idx)?.qhash == qhash {
            run_len += 1;
            idx += 1;
        }

        distinct_hashes += 1;
        max_records_per_hash = max_records_per_hash.max(run_len);
        match run_len {
            1 => singleton_hashes += 1,
            2 => pair_hashes += 1,
            _ => multi_hashes += 1,
        }
    }

    let mean_records_per_hash = if distinct_hashes == 0 {
        0.0
    } else {
        records as f64 / distinct_hashes as f64
    };

    Ok(QnameHashStats {
        records,
        distinct_hashes,
        singleton_hashes,
        pair_hashes,
        multi_hashes,
        max_records_per_hash,
        mean_records_per_hash,
    })
}

fn print_stats_text(
    input_bam: &str,
    index_path: &str,
    index_size: u64,
    index: &Index,
    stats: &QnameHashStats,
) -> Result<()> {
    let metadata = index
        .bam_metadata()
        .ok_or_else(|| "[qbix] index metadata is unavailable".to_string())?;
    println!("Records:\t{}", stats.records);
    println!("Distinct read-name hashes:\t{}", stats.distinct_hashes);
    println!("Records per name:");
    println!(
        "  1 (singletons):\t{} ({:.1}%)",
        stats.singleton_hashes,
        percent(stats.singleton_hashes, stats.distinct_hashes)
    );
    println!(
        "  2 (pairs):\t{} ({:.1}%)",
        stats.pair_hashes,
        percent(stats.pair_hashes, stats.distinct_hashes)
    );
    println!(
        "  3+ (multi/suppl.):\t{} ({:.1}%)",
        stats.multi_hashes,
        percent(stats.multi_hashes, stats.distinct_hashes)
    );
    println!("  max:\t{}", stats.max_records_per_hash);
    println!("  mean:\t{:.2}", stats.mean_records_per_hash);
    println!("Index metadata:");
    println!("  BAM:\t{input_bam}");
    println!("  Index:\t{index_path}");
    println!("  Format:\tQBI1");
    println!("  Index size:\t{index_size}");
    println!("  BAM size:\t{}", metadata.size());
    println!("  BAM mtime ns:\t{}", metadata.mtime());
    println!("  Header hash:\t0x{:016x}", metadata.header_hash());
    Ok(())
}

fn print_stats_json(
    input_bam: &str,
    index_path: &str,
    index_size: u64,
    index: &Index,
    stats: &QnameHashStats,
) -> Result<()> {
    let metadata = index
        .bam_metadata()
        .ok_or_else(|| "[qbix] index metadata is unavailable".to_string())?;
    println!("{{");
    println!("  \"bam\": \"{}\",", json_escape(input_bam));
    println!("  \"index\": \"{}\",", json_escape(index_path));
    println!("  \"format\": \"QBI1\",");
    println!("  \"records\": {},", stats.records);
    println!("  \"distinct_qname_hashes\": {},", stats.distinct_hashes);
    println!("  \"records_per_name\": {{");
    println!("    \"singletons\": {},", stats.singleton_hashes);
    println!("    \"pairs\": {},", stats.pair_hashes);
    println!("    \"multi_or_supplementary\": {},", stats.multi_hashes);
    println!("    \"max\": {},", stats.max_records_per_hash);
    println!("    \"mean\": {:.6}", stats.mean_records_per_hash);
    println!("  }},");
    println!("  \"index_size\": {index_size},");
    println!("  \"bam_size\": {},", metadata.size());
    println!("  \"bam_mtime_ns\": {},", metadata.mtime());
    println!("  \"header_hash\": \"0x{:016x}\"", metadata.header_hash());
    println!("}}");
    Ok(())
}

fn percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 100.0 / total as f64
    }
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}
