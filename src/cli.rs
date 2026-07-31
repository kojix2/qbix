use crate::commands::{self, CheckMode, ColorMode, GetOrder, OutputFormat, StatsFormat};
use crate::error::Result;
use crate::index::{
    DEFAULT_BUCKET_BITS, DEFAULT_INDEX_MEMORY_LIMIT, DEFAULT_SORT_THREADS, MAX_BUCKET_BITS,
    MIN_BUCKET_BITS,
};
use crate::VERSION;
use clap::{error::ErrorKind, Arg, ArgAction, ArgMatches, Command};
use std::io::BufRead;
use std::io::Write;

const APP_NAME: &str = "qbix";
const COMMAND_INDEX: &str = "index";
const COMMAND_GET: &str = "get";
const COMMAND_SHOW: &str = "show";
const COMMAND_CHECK: &str = "check";
const COMMAND_STATS: &str = "stats";
const COMMAND_STAT: &str = "stat";
const ARG_INDEX: &str = "index";
const ARG_INPUT_BAM: &str = "input_bam";
const ARG_INPUT_INDEX: &str = "input_index";
const ARG_READNAMES: &str = "readnames";
const ARG_THREADS: &str = "threads";
const ARG_MEMORY: &str = "memory";
const ARG_BUCKET_BITS: &str = "bucket_bits";
const ARG_SORT_THREADS: &str = "sort_threads";
const ARG_TEMP_DIR: &str = "temp_dir";
const ARG_VERBOSE: &str = "verbose";
const ARG_BAM_ORDER: &str = "bam_order";
const ARG_QUERY_ORDER: &str = "query_order";
const ARG_READNAMES_FILE: &str = "readnames_file";
const ARG_UNIQUE: &str = "unique";
const ARG_MISSING: &str = "missing";
const ARG_WITH_HEADER: &str = "with_header";
const ARG_OUTPUT_BAM: &str = "output_bam";
const ARG_OUTPUT_FORMAT: &str = "output_format";
const ARG_OUTPUT: &str = "output";
#[cfg(feature = "biosyntax")]
const ARG_COLOR: &str = "color";
const ARG_QUICK: &str = "quick";
const ARG_FULL: &str = "full";
const ARG_JSON: &str = "json";
const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");
const TOP_LEVEL_HELP_TEMPLATE: &str = "\
Program: qbix
Version: {version}
Source:  {author}

Usage:   {usage}

Commands:
{subcommands}

General options:
{options}";

pub fn run<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    match parse_args(args)? {
        Action::Index {
            input_bam,
            output_index,
            verbose,
            threads,
            memory_limit,
            bucket_bits,
            sort_threads,
            temp_dir,
        } => commands::build_index(
            &input_bam,
            commands::BuildIndexOptions {
                output_index: output_index.as_deref(),
                verbose,
                threads,
                memory_limit,
                bucket_bits,
                sort_threads,
                temp_dir: temp_dir.as_deref(),
            },
        ),
        Action::Get(action) => run_get(action),
        Action::Show { input_index } => commands::show_index(&input_index),
        Action::Check {
            input_bam,
            input_index,
            threads,
            verbose,
            mode,
        } => commands::check_index(&input_bam, input_index.as_deref(), threads, verbose, mode),
        Action::Stats {
            input_bam,
            input_index,
            format,
        } => commands::stats_index(&input_bam, input_index.as_deref(), format),
        Action::HelpDisplayed => Ok(()),
    }
}

fn parse_args<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if let Some(mut command) = help_command(&args) {
        write_command_help_stdout(&mut command)?;
        return Ok(Action::HelpDisplayed);
    }
    if args.len() == 1 {
        write_command_help_stderr(&mut app())?;
        return Err("[qbix] no subcommand provided".to_string());
    }

    let subcommand_name = args.get(1).cloned();
    let matches = match app().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            err.print()
                .map_err(|e| format!("[qbix] could not write help text: {e}"))?;
            return Ok(Action::HelpDisplayed);
        }
        Err(err) if err.kind() == ErrorKind::MissingRequiredArgument => {
            if let Some(command_name) = subcommand_name.as_deref() {
                print_subcommand_help(command_name)?;
            }
            return Err(prefix_error(&err));
        }
        Err(err) => return Err(prefix_error(&err)),
    };

    action_from_matches(&matches)
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Index {
        input_bam: String,
        output_index: Option<String>,
        verbose: bool,
        threads: usize,
        memory_limit: usize,
        bucket_bits: u8,
        sort_threads: usize,
        temp_dir: Option<String>,
    },
    Get(GetAction),
    Show {
        input_index: String,
    },
    Check {
        input_bam: String,
        input_index: Option<String>,
        threads: usize,
        verbose: bool,
        mode: CheckMode,
    },
    Stats {
        input_bam: String,
        input_index: Option<String>,
        format: StatsFormat,
    },
    HelpDisplayed,
}

#[derive(Debug, PartialEq, Eq)]
struct GetAction {
    input_bam: String,
    input_index: Option<String>,
    readnames: Vec<String>,
    readnames_file: Option<String>,
    unique: bool,
    with_header: bool,
    threads: usize,
    order: GetOrder,
    output_format: OutputFormat,
    output_path: Option<String>,
    missing_path: Option<String>,
    color_mode: ColorMode,
}

fn action_from_matches(matches: &ArgMatches) -> Result<Action> {
    match matches.subcommand() {
        Some((COMMAND_INDEX, matches)) => Ok(Action::Index {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            output_index: optional_string(matches, ARG_INDEX),
            verbose: matches.get_flag(ARG_VERBOSE),
            threads: threads(matches, default_bgzf_threads())?,
            memory_limit: memory_limit(matches)?,
            bucket_bits: bucket_bits(matches)?,
            sort_threads: sort_threads(matches)?,
            temp_dir: optional_string(matches, ARG_TEMP_DIR),
        }),
        Some((COMMAND_GET, matches)) => Ok(Action::Get(GetAction {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            input_index: optional_string(matches, ARG_INDEX),
            readnames: optional_values(matches, ARG_READNAMES),
            readnames_file: optional_string(matches, ARG_READNAMES_FILE),
            unique: matches.get_flag(ARG_UNIQUE),
            with_header: matches.get_flag(ARG_WITH_HEADER),
            threads: threads(matches, 1)?,
            order: get_order(matches),
            output_format: output_format(matches)?,
            output_path: optional_string(matches, ARG_OUTPUT),
            missing_path: optional_string(matches, ARG_MISSING),
            color_mode: color_mode(matches)?,
        })),
        Some((COMMAND_SHOW, matches)) => Ok(Action::Show {
            input_index: required_string(matches, ARG_INPUT_INDEX)?.to_string(),
        }),
        Some((COMMAND_CHECK, matches)) => Ok(Action::Check {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            input_index: optional_string(matches, ARG_INDEX),
            threads: threads(matches, 1)?,
            verbose: matches.get_flag(ARG_VERBOSE),
            mode: check_mode(matches),
        }),
        Some((COMMAND_STATS | COMMAND_STAT, matches)) => Ok(Action::Stats {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            input_index: optional_string(matches, ARG_INDEX),
            format: stats_format(matches),
        }),
        _ => Err("[qbix] usage qbix <COMMAND> [...]".to_string()),
    }
}

fn app() -> Command {
    Command::new(APP_NAME)
        .about("Index and retrieve BAM records by QNAME")
        .author(SOURCE_URL)
        .version(VERSION)
        .override_usage("qbix <command> [options]")
        .help_template(TOP_LEVEL_HELP_TEMPLATE)
        .disable_help_subcommand(true)
        .subcommand_required(true)
        .subcommand(index_command())
        .subcommand(get_command())
        .subcommand(show_command())
        .subcommand(check_command())
        .subcommand(stats_command())
}

fn index_command() -> Command {
    Command::new(COMMAND_INDEX)
        .about("Build a QNAME index for a BAM file")
        .arg(index_arg())
        .arg(index_threads_arg())
        .arg(memory_arg())
        .arg(bucket_bits_arg())
        .arg(sort_threads_arg())
        .arg(temp_dir_arg())
        .arg(verbose_arg())
        .arg(input_bam_arg())
}

fn get_command() -> Command {
    let command = Command::new(COMMAND_GET)
        .about("Retrieve BAM records by QNAME")
        .arg(index_arg())
        .arg(threads_arg())
        .arg(
            Arg::new(ARG_BAM_ORDER)
                .long("bam-order")
                .action(ArgAction::SetTrue)
                .help("Emit records in BAM order")
                .conflicts_with(ARG_QUERY_ORDER),
        )
        .arg(
            Arg::new(ARG_QUERY_ORDER)
                .long("query-order")
                .action(ArgAction::SetTrue)
                .help("Emit records in query order")
                .conflicts_with(ARG_BAM_ORDER),
        )
        .arg(
            Arg::new(ARG_READNAMES_FILE)
                .short('f')
                .long("file")
                .value_name("readnames.txt")
                .help("Read names from a file, or '-' for stdin"),
        )
        .arg(
            Arg::new(ARG_UNIQUE)
                .long("unique")
                .action(ArgAction::SetTrue)
                .help("Process each input read name only once"),
        )
        .arg(
            Arg::new(ARG_MISSING)
                .long("missing")
                .value_name("missing.txt")
                .help("Write query QNAMEs with no matching BAM record to a file"),
        )
        .arg(
            Arg::new(ARG_WITH_HEADER)
                .long("with-header")
                .action(ArgAction::SetTrue)
                .help("Include the source BAM header in SAM output"),
        )
        .arg(
            Arg::new(ARG_OUTPUT_BAM)
                .short('b')
                .long("bam")
                .action(ArgAction::SetTrue)
                .help("Output BAM"),
        )
        .arg(
            Arg::new(ARG_OUTPUT_FORMAT)
                .short('O')
                .long("output-fmt")
                .value_name("SAM|BAM")
                .default_value("SAM")
                .help("Output format"),
        )
        .arg(
            Arg::new(ARG_OUTPUT)
                .short('o')
                .long("output")
                .value_name("output")
                .help("Output path, or '-' for stdout"),
        );
    #[cfg(feature = "biosyntax")]
    let command = command.arg(
        Arg::new(ARG_COLOR)
            .long("color")
            .value_name("auto|always|never")
            .default_value("auto")
            .help("Color SAM output when libbiosyntax support is enabled"),
    );
    command.arg(input_bam_arg()).arg(readnames_arg())
}

fn show_command() -> Command {
    Command::new(COMMAND_SHOW)
        .about("Print raw QBI index rows")
        .arg(input_index_arg())
}

fn check_command() -> Command {
    Command::new(COMMAND_CHECK)
        .about("Check a QBI index against its BAM file")
        .arg(index_arg())
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(
            Arg::new(ARG_QUICK)
                .long("quick")
                .action(ArgAction::SetTrue)
                .help("Only check BAM size, mtime, and header hash")
                .conflicts_with(ARG_FULL),
        )
        .arg(
            Arg::new(ARG_FULL)
                .long("full")
                .action(ArgAction::SetTrue)
                .help("Also seek to every indexed record and verify its QNAME hash")
                .conflicts_with(ARG_QUICK),
        )
        .arg(input_bam_arg())
}

fn stats_command() -> Command {
    Command::new(COMMAND_STATS)
        .alias(COMMAND_STAT)
        .about("Print QBI index statistics")
        .arg(index_arg())
        .arg(
            Arg::new(ARG_JSON)
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Print JSON"),
        )
        .arg(input_bam_arg())
}

fn print_subcommand_help(command_name: &str) -> Result<()> {
    let Some(mut command) = command_for_help(command_name) else {
        return Ok(());
    };
    write_command_help_stderr(&mut command)
}

fn write_command_help_stdout(command: &mut Command) -> Result<()> {
    let mut stdout = std::io::stdout();
    write_command_help(command, &mut stdout)
}

fn write_command_help_stderr(command: &mut Command) -> Result<()> {
    let mut stderr = std::io::stderr();
    write_command_help(command, &mut stderr)
}

fn write_command_help<W>(command: &mut Command, writer: &mut W) -> Result<()>
where
    W: Write,
{
    writeln!(writer).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    command
        .write_help(writer)
        .map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    writeln!(writer).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    Ok(())
}

fn help_command(args: &[String]) -> Option<Command> {
    match args {
        [_, flag] if is_help_flag(flag) => Some(app()),
        [_, command_name, flag] if is_help_flag(flag) => command_for_help(command_name),
        _ => None,
    }
}

fn command_for_help(command_name: &str) -> Option<Command> {
    let command = subcommand(command_name)?;
    Some(command.bin_name(format!("{APP_NAME} {command_name}")))
}

fn is_help_flag(value: &str) -> bool {
    value == "-h" || value == "--help"
}

fn prefix_error(err: &clap::Error) -> String {
    let message = err
        .to_string()
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("argument error")
        .trim_start_matches("error: ")
        .to_string();
    format!("[qbix] {message}")
}

fn subcommand(name: &str) -> Option<Command> {
    match name {
        COMMAND_INDEX => Some(index_command()),
        COMMAND_GET => Some(get_command()),
        COMMAND_SHOW => Some(show_command()),
        COMMAND_CHECK => Some(check_command()),
        COMMAND_STATS | COMMAND_STAT => Some(stats_command()),
        _ => None,
    }
}

fn index_arg() -> Arg {
    Arg::new(ARG_INDEX)
        .short('i')
        .long("index")
        .value_name("index.qbi")
        .help("QBI index path")
}

fn input_bam_arg() -> Arg {
    Arg::new(ARG_INPUT_BAM)
        .value_name("input.bam")
        .help("Input BAM file")
        .required(true)
}

fn input_index_arg() -> Arg {
    Arg::new(ARG_INPUT_INDEX)
        .value_name("input.qbi")
        .help("Input QBI index file")
        .required(true)
}

fn readnames_arg() -> Arg {
    Arg::new(ARG_READNAMES)
        .value_name("readname")
        .help("Read name to fetch")
        .required_unless_present(ARG_READNAMES_FILE)
        .num_args(1..)
}

// `index` reads the BAM sequentially. More BGZF threads make it faster.
// `get` and `check` do scattered single-record seeks instead. A thread
// pool adds overhead there with no streaming gain to offset it, so more
// threads make them slower (measured, see benchmark notes). This cap
// applies only to `index`.
//
// Many bioinformatics CLIs default I/O threads to 1. This keeps a tool
// from using more of a shared or HPC node than the user asked for.
//
// Measured on chr21 BAM subsets (PacBio/ONT): 8 threads still give good
// efficiency (75-80%). Past that, gains drop fast: 60% at 12 threads,
// flat from 12 to 16. 8 is the best trade-off, without assuming a
// generous default core count.
const DEFAULT_BGZF_THREADS_CAP: usize = 8;

fn default_bgzf_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(DEFAULT_BGZF_THREADS_CAP))
        .unwrap_or(1)
}

fn threads_arg() -> Arg {
    Arg::new(ARG_THREADS)
        .short('@')
        .long("bgzf-threads")
        .alias("threads")
        .value_name("INT")
        .help("Number of BGZF/htslib I/O threads [default: 1]")
}

fn index_threads_arg() -> Arg {
    threads_arg().help("Number of BGZF/htslib I/O threads [default: min(available cores, 8)]")
}

fn memory_arg() -> Arg {
    Arg::new(ARG_MEMORY)
        .long("memory")
        .value_name("SIZE")
        .default_value("512M")
        .help("Maximum bucket memory while building the index (K, M, or G suffix accepted)")
}

fn bucket_bits_arg() -> Arg {
    Arg::new(ARG_BUCKET_BITS)
        .long("bucket-bits")
        .value_name("INT")
        .default_value("8")
        .help("Bucket prefix bits for index building (advanced)")
}

fn sort_threads_arg() -> Arg {
    Arg::new(ARG_SORT_THREADS)
        .long("sort-threads")
        .value_name("INT")
        .default_value("1")
        .help("Number of bucket sort worker threads; may use up to INT * --memory")
}

fn temp_dir_arg() -> Arg {
    Arg::new(ARG_TEMP_DIR)
        .long("temp-dir")
        .value_name("DIR")
        .help("Directory for bucket temporary files")
}

fn verbose_arg() -> Arg {
    Arg::new(ARG_VERBOSE)
        .short('v')
        .long("verbose")
        .help("Print progress to stderr")
        .action(ArgAction::SetTrue)
}

fn required_string<'a>(matches: &'a ArgMatches, name: &str) -> Result<&'a str> {
    matches
        .get_one::<String>(name)
        .map(String::as_str)
        .ok_or_else(|| format!("[qbix] missing required argument: {name}"))
}

fn optional_string(matches: &ArgMatches, name: &str) -> Option<String> {
    matches.get_one::<String>(name).cloned()
}

fn threads(matches: &ArgMatches, fallback: usize) -> Result<usize> {
    let threads = match optional_string(matches, ARG_THREADS) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "[qbix] threads must be a positive integer".to_string())?,
        None => fallback,
    };
    if threads == 0 {
        return Err("[qbix] threads must be a positive integer".to_string());
    }
    Ok(threads)
}

fn memory_limit(matches: &ArgMatches) -> Result<usize> {
    let value = optional_string(matches, ARG_MEMORY)
        .unwrap_or_else(|| DEFAULT_INDEX_MEMORY_LIMIT.to_string());
    parse_size(&value)
}

fn bucket_bits(matches: &ArgMatches) -> Result<u8> {
    let value = optional_string(matches, ARG_BUCKET_BITS)
        .unwrap_or_else(|| DEFAULT_BUCKET_BITS.to_string());
    let bits = value
        .parse::<u8>()
        .map_err(|_| "[qbix] bucket bits must be a positive integer".to_string())?;
    if !(MIN_BUCKET_BITS..=MAX_BUCKET_BITS).contains(&bits) {
        return Err(format!(
            "[qbix] bucket bits must be between {MIN_BUCKET_BITS} and {MAX_BUCKET_BITS}"
        ));
    }
    Ok(bits)
}

fn sort_threads(matches: &ArgMatches) -> Result<usize> {
    let value = optional_string(matches, ARG_SORT_THREADS)
        .unwrap_or_else(|| DEFAULT_SORT_THREADS.to_string());
    let threads = value
        .parse::<usize>()
        .map_err(|_| "[qbix] sort threads must be a positive integer".to_string())?;
    if threads == 0 {
        return Err("[qbix] sort threads must be a positive integer".to_string());
    }
    Ok(threads)
}

fn parse_size(value: &str) -> Result<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("[qbix] memory size must not be empty".to_string());
    }
    let (number, multiplier) = match trimmed.as_bytes().last().copied() {
        Some(b'K' | b'k') => (&trimmed[..trimmed.len() - 1], 1024usize),
        Some(b'M' | b'm') => (&trimmed[..trimmed.len() - 1], 1024usize * 1024),
        Some(b'G' | b'g') => (&trimmed[..trimmed.len() - 1], 1024usize * 1024 * 1024),
        _ => (trimmed, 1usize),
    };
    let number = number.parse::<usize>().map_err(|_| {
        "[qbix] memory size must be an integer with optional K/M/G suffix".to_string()
    })?;
    let bytes = number
        .checked_mul(multiplier)
        .ok_or_else(|| "[qbix] memory size is too large".to_string())?;
    if bytes == 0 {
        return Err("[qbix] memory size must be positive".to_string());
    }
    Ok(bytes)
}

fn get_order(matches: &ArgMatches) -> GetOrder {
    if matches.get_flag(ARG_BAM_ORDER) {
        GetOrder::Bam
    } else {
        GetOrder::Query
    }
}

fn check_mode(matches: &ArgMatches) -> CheckMode {
    if matches.get_flag(ARG_FULL) {
        CheckMode::Full
    } else {
        CheckMode::Quick
    }
}

fn stats_format(matches: &ArgMatches) -> StatsFormat {
    if matches.get_flag(ARG_JSON) {
        StatsFormat::Json
    } else {
        StatsFormat::Text
    }
}

fn output_format(matches: &ArgMatches) -> Result<OutputFormat> {
    if matches.get_flag(ARG_OUTPUT_BAM) {
        return Ok(OutputFormat::Bam);
    }

    match required_string(matches, ARG_OUTPUT_FORMAT)?
        .to_ascii_uppercase()
        .as_str()
    {
        "S" | "SAM" => Ok(OutputFormat::Sam),
        "B" | "BAM" => Ok(OutputFormat::Bam),
        format => Err(format!(
            "[qbix] unsupported output format: {format}; expected SAM or BAM"
        )),
    }
}

#[cfg(feature = "biosyntax")]
fn color_mode(matches: &ArgMatches) -> Result<ColorMode> {
    match required_string(matches, ARG_COLOR)? {
        "auto" => Ok(ColorMode::Auto),
        "always" => Ok(ColorMode::Always),
        "never" => Ok(ColorMode::Never),
        value => Err(format!(
            "[qbix] unsupported color mode: {value}; expected auto, always, or never"
        )),
    }
}

#[cfg(not(feature = "biosyntax"))]
fn color_mode(_matches: &ArgMatches) -> Result<ColorMode> {
    Ok(ColorMode::Auto)
}

fn optional_values(matches: &ArgMatches, name: &str) -> Vec<String> {
    matches
        .get_many::<String>(name)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn run_get(action: GetAction) -> Result<()> {
    let GetAction {
        input_bam,
        input_index,
        readnames,
        readnames_file,
        unique,
        with_header,
        threads,
        order,
        output_format,
        output_path,
        missing_path,
        color_mode,
    } = action;
    let options = commands::GetOptions {
        input_index: input_index.as_deref(),
        threads,
        order,
        unique,
        with_header,
        output_format,
        output_path: output_path.as_deref(),
        missing_path: missing_path.as_deref(),
        readnames_path: readnames_file.as_deref().filter(|path| *path != "-"),
        color_mode,
    };
    let positional = readnames.into_iter().map(Ok);

    // Keep file and stdin input lazy here. In query order, get_records consumes
    // this chain one QNAME at a time, so a long-running generator can produce
    // results before EOF without storing its complete output in memory.
    match readnames_file.as_deref() {
        None => commands::get_records(&input_bam, positional, options),
        Some("-") => {
            let stdin = std::io::stdin();
            let from_file = readnames_from_reader(stdin.lock());
            commands::get_records(&input_bam, positional.chain(from_file), options)
        }
        Some(path) => {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("[qbix] could not open read names file {path}: {e}"))?;
            let from_file = readnames_from_reader(std::io::BufReader::new(file));
            commands::get_records(&input_bam, positional.chain(from_file), options)
        }
    }
}

fn readnames_from_reader<R>(reader: R) -> impl Iterator<Item = Result<String>>
where
    R: BufRead,
{
    reader.lines().filter_map(|line| match line {
        Ok(readname) if readname.is_empty() => None,
        Ok(readname) => Some(Ok(readname)),
        Err(e) => Some(Err(format!("[qbix] could not read read names: {e}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_options() {
        let action = parse_args(strings([
            "qbix",
            "index",
            "-v",
            "-i",
            "reads.qbi",
            "reads.bam",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Index {
                input_bam: "reads.bam".to_string(),
                output_index: Some("reads.qbi".to_string()),
                verbose: true,
                threads: default_bgzf_threads(),
                memory_limit: DEFAULT_INDEX_MEMORY_LIMIT,
                bucket_bits: DEFAULT_BUCKET_BITS,
                sort_threads: DEFAULT_SORT_THREADS,
                temp_dir: None,
            }
        );
    }

    #[test]
    fn parses_index_build_options() {
        let action = parse_args(strings([
            "qbix",
            "index",
            "--memory",
            "2G",
            "--bucket-bits",
            "10",
            "--sort-threads",
            "3",
            "--temp-dir",
            "tmp",
            "reads.bam",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Index {
                input_bam: "reads.bam".to_string(),
                output_index: None,
                verbose: false,
                threads: default_bgzf_threads(),
                memory_limit: 2 * 1024 * 1024 * 1024,
                bucket_bits: 10,
                sort_threads: 3,
                temp_dir: Some("tmp".to_string()),
            }
        );
    }

    #[test]
    fn parses_get_readnames() {
        let action = parse_args(strings([
            "qbix",
            "get",
            "-@",
            "4",
            "reads.bam",
            "read1",
            "read2",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Get(GetAction {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string(), "read2".to_string()],
                readnames_file: None,
                unique: false,
                with_header: false,
                threads: 4,
                order: GetOrder::Query,
                output_format: OutputFormat::Sam,
                output_path: None,
                missing_path: None,
                color_mode: ColorMode::Auto,
            })
        );
    }

    #[test]
    fn parses_get_output_options() {
        let action = parse_args(strings([
            "qbix",
            "get",
            "reads.bam",
            "-Ob",
            "-o",
            "hits.bam",
            "--missing",
            "missing.txt",
            "--with-header",
            "read1",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Get(GetAction {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string()],
                readnames_file: None,
                unique: false,
                with_header: true,
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Bam,
                output_path: Some("hits.bam".to_string()),
                missing_path: Some("missing.txt".to_string()),
                color_mode: ColorMode::Auto,
            })
        );
    }

    #[test]
    fn parses_get_bam_shortcut() {
        let action = parse_args(strings(["qbix", "get", "reads.bam", "-b", "read1"])).unwrap();

        assert_eq!(
            action,
            Action::Get(GetAction {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string()],
                readnames_file: None,
                unique: false,
                with_header: false,
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Bam,
                output_path: None,
                missing_path: None,
                color_mode: ColorMode::Auto,
            })
        );
    }

    #[test]
    fn parses_get_readnames_from_positional_and_file() {
        let action = parse_args(strings([
            "qbix",
            "get",
            "reads.bam",
            "read1",
            "-f",
            "names.txt",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Get(GetAction {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string()],
                readnames_file: Some("names.txt".to_string()),
                unique: false,
                with_header: false,
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Sam,
                output_path: None,
                missing_path: None,
                color_mode: ColorMode::Auto,
            })
        );
    }

    #[test]
    fn rejects_zero_threads() {
        let err = parse_args(strings([
            "qbix",
            "index",
            "--bgzf-threads",
            "0",
            "reads.bam",
        ]))
        .unwrap_err();
        assert!(err.contains("positive integer"));
    }

    #[test]
    fn default_bgzf_threads_stays_within_cap() {
        let threads = default_bgzf_threads();
        assert!(threads >= 1);
        assert!(threads <= DEFAULT_BGZF_THREADS_CAP);
    }

    #[test]
    fn accepts_threads_alias() {
        let action = parse_args(strings(["qbix", "index", "--threads", "2", "reads.bam"])).unwrap();

        assert_eq!(
            action,
            Action::Index {
                input_bam: "reads.bam".to_string(),
                output_index: None,
                verbose: false,
                threads: 2,
                memory_limit: DEFAULT_INDEX_MEMORY_LIMIT,
                bucket_bits: DEFAULT_BUCKET_BITS,
                sort_threads: DEFAULT_SORT_THREADS,
                temp_dir: None,
            }
        );
    }

    #[test]
    fn rejects_zero_sort_threads() {
        let err = parse_args(strings([
            "qbix",
            "index",
            "--sort-threads",
            "0",
            "reads.bam",
        ]))
        .unwrap_err();
        assert!(err.contains("sort threads"));
        assert!(err.contains("positive integer"));
    }

    #[test]
    fn parses_check_options() {
        let action = parse_args(strings([
            "qbix",
            "check",
            "-v",
            "-@",
            "2",
            "--full",
            "reads.bam",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Check {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                threads: 2,
                verbose: true,
                mode: CheckMode::Full,
            }
        );
    }

    #[test]
    fn check_defaults_to_quick() {
        let action = parse_args(strings(["qbix", "check", "reads.bam"])).unwrap();

        assert_eq!(
            action,
            Action::Check {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                threads: 1,
                verbose: false,
                mode: CheckMode::Quick,
            }
        );
    }

    #[test]
    fn parses_stats_options() {
        let action = parse_args(strings([
            "qbix",
            "stats",
            "-i",
            "reads.qbi",
            "--json",
            "reads.bam",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Stats {
                input_bam: "reads.bam".to_string(),
                input_index: Some("reads.qbi".to_string()),
                format: StatsFormat::Json,
            }
        );
    }

    #[test]
    fn parses_stat_alias() {
        let action = parse_args(strings(["qbix", "stat", "reads.bam"])).unwrap();

        assert_eq!(
            action,
            Action::Stats {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                format: StatsFormat::Text,
            }
        );
    }

    #[test]
    fn rejects_get_without_readname() {
        let err = parse_args(strings(["qbix", "get", "reads.bam"])).unwrap_err();
        assert!(err.contains("required"));
    }

    #[test]
    fn accepts_version_flag() {
        let action = parse_args(strings(["qbix", "--version"])).unwrap();
        assert_eq!(action, Action::HelpDisplayed);
    }

    #[test]
    fn help_lists_subcommand_descriptions() {
        let mut app = app();
        let help = app.render_help().to_string();

        assert!(help.contains("Build a QNAME index for a BAM file"));
        assert!(help.contains("Retrieve BAM records by QNAME"));
        assert!(help.contains("Print raw QBI index rows"));
        assert!(help.contains("Check a QBI index against its BAM file"));
        assert!(help.contains("Print QBI index statistics"));
    }

    fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_string).collect()
    }
}
