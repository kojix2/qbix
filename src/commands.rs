use crate::error::Result;
use crate::hts::{BamRecord, Header, HtsFile};
use crate::index::{generate_index_filename, qname_hash64, BamMetadata, BucketIndexBuilder, Index};
use std::io::IsTerminal;
#[cfg(feature = "biosyntax")]
use std::io::Write;

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
pub(crate) enum ColorMode {
    Auto,
    Always,
    Never,
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

pub(crate) struct GetOptions<'a> {
    pub(crate) input_index: Option<&'a str>,
    pub(crate) threads: usize,
    pub(crate) order: GetOrder,
    pub(crate) output_format: OutputFormat,
    pub(crate) output_path: Option<&'a str>,
    pub(crate) color_mode: ColorMode,
}

pub(crate) struct BuildIndexOptions<'a> {
    pub(crate) output_index: Option<&'a str>,
    pub(crate) verbose: bool,
    pub(crate) threads: usize,
    pub(crate) memory_limit: usize,
    pub(crate) bucket_bits: u8,
    pub(crate) sort_threads: usize,
    pub(crate) temp_dir: Option<&'a str>,
}

pub(crate) fn build_index(input_bam: &str, options: BuildIndexOptions<'_>) -> Result<()> {
    let bam =
        HtsFile::open(input_bam, "r").map_err(|_| format!("[qbix] could not open {input_bam}"))?;
    bam.set_threads(options.threads)?;
    let header = bam
        .read_header()
        .map_err(|_| format!("[qbix] could not read BAM header from {input_bam}"))?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)?;
    let rec = BamRecord::new()?;
    let out_fn = generate_index_filename(Some(input_bam), options.output_index)?;
    let mut builder = BucketIndexBuilder::new(
        &out_fn,
        options.memory_limit,
        options.bucket_bits,
        options.sort_threads,
        options.temp_dir,
    )?;
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
        let record = builder.add(readname, file_offset)?;
        if options.verbose
            && (builder.total_records() == 1 || builder.total_records().is_multiple_of(100_000))
        {
            eprintln!(
                "[qbix] build: record {} [{} {}] {}",
                builder.total_records(),
                record.qhash,
                record.file_offset,
                readname
            );
        }
        file_offset = bam.tell()?;
    }

    if options.verbose {
        eprintln!("[qbix] build: writing to disk...");
    }
    let total_records = builder.total_records();
    builder.finish(bam_metadata)?;
    if options.verbose {
        eprintln!("[qbix] build: wrote index for {total_records} records.");
    }
    Ok(())
}

pub(crate) fn get_records(
    input_bam: &str,
    readnames: &[String],
    options: GetOptions<'_>,
) -> Result<()> {
    let bam = HtsFile::open(input_bam, "r")
        .map_err(|_| format!("[qbix] could not open BAM file: {input_bam}"))?;
    bam.set_threads(options.threads)?;
    bam.set_bgzf_cache_size(BGZF_CACHE_SIZE)?;
    let header = bam
        .read_header()
        .map_err(|_| format!("[qbix] could not read BAM header: {input_bam}"))?;
    let bam_metadata = BamMetadata::from_bam(input_bam, header.text_hash()?)?;
    let index = Index::load(Some(input_bam), options.input_index, Some(bam_metadata))?;
    let output_path = options.output_path.unwrap_or("-");
    let rec = BamRecord::new()?;
    let mut out = RecordWriter::open(
        output_path,
        options.output_format,
        options.color_mode,
        options.threads,
        &header,
    )?;

    if options.order == GetOrder::Query {
        return write_hits_in_query_order(&bam, &header, &mut out, &rec, &index, readnames);
    }

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
    hits.sort_by_key(|hit| hit.file_offset);

    for hit in hits {
        bam.read_record_at(&header, &rec, hit.file_offset)?;
        if rec.qname()? == hit.readname {
            out.write_record(&header, &rec)?;
        }
    }
    Ok(())
}

fn write_hits_in_query_order(
    bam: &HtsFile,
    header: &Header,
    out: &mut RecordWriter,
    rec: &BamRecord,
    index: &Index,
    readnames: &[String],
) -> Result<()> {
    for readname in readnames {
        for idx in index.range_indices(readname)? {
            let record = index.record(idx)?;
            bam.read_record_at(header, rec, record.file_offset)?;
            if rec.qname()? == readname {
                out.write_record(header, rec)?;
            }
        }
    }
    Ok(())
}

enum RecordWriter {
    Hts(HtsFile),
    #[cfg(feature = "biosyntax")]
    Colored(ColorSamWriter),
}

impl RecordWriter {
    fn open(
        output_path: &str,
        output_format: OutputFormat,
        color_mode: ColorMode,
        threads: usize,
        header: &Header,
    ) -> Result<Self> {
        if should_color(output_format, color_mode, output_path) {
            #[cfg(feature = "biosyntax")]
            {
                return Ok(Self::Colored(ColorSamWriter::open(output_path)?));
            }
            #[cfg(not(feature = "biosyntax"))]
            {
                return Err(
                    "[qbix] colored SAM output requires building with --features biosyntax"
                        .to_string(),
                );
            }
        }

        let out = HtsFile::open(output_path, output_format.hts_mode()).map_err(|_| {
            format!(
                "[qbix] could not open {} output: {output_path}",
                output_format.name()
            )
        })?;
        if output_format == OutputFormat::Bam {
            out.set_threads(threads)?;
            out.write_header(header)?;
        }
        Ok(Self::Hts(out))
    }

    fn write_record(&mut self, header: &Header, rec: &BamRecord) -> Result<()> {
        match self {
            Self::Hts(out) => out.write_record(header, rec),
            #[cfg(feature = "biosyntax")]
            Self::Colored(out) => out.write_record(header, rec),
        }
    }
}

#[cfg(feature = "biosyntax")]
struct ColorSamWriter {
    writer: std::io::BufWriter<Box<dyn std::io::Write>>,
    line_no: u64,
}

#[cfg(feature = "biosyntax")]
impl ColorSamWriter {
    fn open(output_path: &str) -> Result<Self> {
        let writer: Box<dyn std::io::Write> = if output_path == "-" {
            Box::new(std::io::stdout())
        } else {
            Box::new(
                std::fs::File::create(output_path)
                    .map_err(|e| format!("[qbix] could not open SAM output: {output_path}: {e}"))?,
            )
        };
        Ok(Self {
            writer: std::io::BufWriter::new(writer),
            line_no: 0,
        })
    }

    fn write_record(&mut self, header: &Header, rec: &BamRecord) -> Result<()> {
        let line = rec.format_sam(header)?;
        let rendered = crate::biosyntax::render_sam_ansi(&line, self.line_no)?;
        self.writer
            .write_all(&rendered)
            .map_err(|e| format!("[qbix] could not write colored SAM output: {e}"))?;
        self.writer
            .write_all(b"\n")
            .map_err(|e| format!("[qbix] could not write colored SAM output: {e}"))?;
        self.line_no += 1;
        Ok(())
    }
}

fn should_color(output_format: OutputFormat, color_mode: ColorMode, output_path: &str) -> bool {
    should_color_with_terminal(
        output_format,
        color_mode,
        output_path,
        std::io::stdout().is_terminal(),
    )
}

fn should_color_with_terminal(
    output_format: OutputFormat,
    color_mode: ColorMode,
    output_path: &str,
    stdout_is_terminal: bool,
) -> bool {
    if output_format != OutputFormat::Sam {
        return false;
    }
    match color_mode {
        ColorMode::Never => false,
        ColorMode::Always => true,
        ColorMode::Auto => cfg!(feature = "biosyntax") && output_path == "-" && stdout_is_terminal,
    }
}

#[cfg(test)]
mod tests {
    use super::{should_color_with_terminal, ColorMode, OutputFormat};

    #[test]
    #[cfg(not(feature = "biosyntax"))]
    fn auto_color_falls_back_to_plain_without_biosyntax_even_on_terminal() {
        assert!(!should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Auto,
            "-",
            true,
        ));
    }

    #[test]
    #[cfg(feature = "biosyntax")]
    fn auto_color_uses_biosyntax_for_terminal_sam_stdout() {
        assert!(should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Auto,
            "-",
            true,
        ));
    }

    #[test]
    fn auto_color_leaves_pipes_files_and_bam_plain() {
        assert!(!should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Auto,
            "-",
            false,
        ));
        assert!(!should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Auto,
            "hits.sam",
            true,
        ));
        assert!(!should_color_with_terminal(
            OutputFormat::Bam,
            ColorMode::Auto,
            "-",
            true,
        ));
    }

    #[test]
    fn explicit_color_modes_do_not_depend_on_terminal_detection() {
        assert!(should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Always,
            "-",
            false,
        ));
        assert!(!should_color_with_terminal(
            OutputFormat::Sam,
            ColorMode::Never,
            "-",
            true,
        ));
    }
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
