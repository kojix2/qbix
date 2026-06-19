use crate::commands::{self, GetOrder, OutputFormat};
use crate::error::Result;
use crate::VERSION;
use clap::{error::ErrorKind, Arg, ArgAction, ArgMatches, Command};
use std::io::BufRead;
use std::io::Write;

const APP_NAME: &str = "qbix";
const COMMAND_INDEX: &str = "index";
const COMMAND_GET: &str = "get";
const COMMAND_SHOW: &str = "show";
const COMMAND_CHECK: &str = "check";
const ARG_INDEX: &str = "index";
const ARG_INPUT_BAM: &str = "input_bam";
const ARG_INPUT_INDEX: &str = "input_index";
const ARG_READNAMES: &str = "readnames";
const ARG_THREADS: &str = "threads";
const ARG_VERBOSE: &str = "verbose";
const ARG_BAM_ORDER: &str = "bam_order";
const ARG_QUERY_ORDER: &str = "query_order";
const ARG_READNAMES_FILE: &str = "readnames_file";
const ARG_OUTPUT_BAM: &str = "output_bam";
const ARG_OUTPUT_FORMAT: &str = "output_format";
const ARG_OUTPUT: &str = "output";
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
        } => commands::build_index(&input_bam, output_index.as_deref(), verbose, threads),
        Action::Get {
            input_bam,
            input_index,
            readnames,
            threads,
            order,
            output_format,
            output_path,
        } => commands::get_records(
            &input_bam,
            input_index.as_deref(),
            &readnames,
            threads,
            order,
            output_format,
            output_path.as_deref(),
        ),
        Action::Show { input_index } => commands::show_index(&input_index),
        Action::Check {
            input_bam,
            input_index,
            threads,
            verbose,
        } => commands::check_index(&input_bam, input_index.as_deref(), threads, verbose),
        Action::HelpDisplayed => Ok(()),
    }
}

fn parse_args<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if let Some(mut command) = help_command(&args) {
        write_command_help(&mut command)?;
        return Ok(Action::HelpDisplayed);
    }
    if args.len() == 1 {
        write_command_help(&mut app())?;
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
    },
    Get {
        input_bam: String,
        input_index: Option<String>,
        readnames: Vec<String>,
        threads: usize,
        order: GetOrder,
        output_format: OutputFormat,
        output_path: Option<String>,
    },
    Show {
        input_index: String,
    },
    Check {
        input_bam: String,
        input_index: Option<String>,
        threads: usize,
        verbose: bool,
    },
    HelpDisplayed,
}

fn action_from_matches(matches: &ArgMatches) -> Result<Action> {
    match matches.subcommand() {
        Some((COMMAND_INDEX, matches)) => Ok(Action::Index {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            output_index: optional_string(matches, ARG_INDEX),
            verbose: matches.get_flag(ARG_VERBOSE),
            threads: threads(matches)?,
        }),
        Some((COMMAND_GET, matches)) => Ok(Action::Get {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            input_index: optional_string(matches, ARG_INDEX),
            readnames: get_readnames(matches)?,
            threads: threads(matches)?,
            order: get_order(matches),
            output_format: output_format(matches)?,
            output_path: optional_string(matches, ARG_OUTPUT),
        }),
        Some((COMMAND_SHOW, matches)) => Ok(Action::Show {
            input_index: required_string(matches, ARG_INPUT_INDEX)?.to_string(),
        }),
        Some((COMMAND_CHECK, matches)) => Ok(Action::Check {
            input_bam: required_string(matches, ARG_INPUT_BAM)?.to_string(),
            input_index: optional_string(matches, ARG_INDEX),
            threads: threads(matches)?,
            verbose: matches.get_flag(ARG_VERBOSE),
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
}

fn index_command() -> Command {
    Command::new(COMMAND_INDEX)
        .about("Build a QNAME index for a BAM file")
        .arg(index_arg())
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(input_bam_arg())
}

fn get_command() -> Command {
    Command::new(COMMAND_GET)
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
                .help("Read read names from a file, or '-' for stdin"),
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
        )
        .arg(input_bam_arg())
        .arg(readnames_arg())
}

fn show_command() -> Command {
    Command::new(COMMAND_SHOW)
        .about("Print raw QBI index rows")
        .arg(input_index_arg())
}

fn check_command() -> Command {
    Command::new(COMMAND_CHECK)
        .about("Validate a QBI index against its BAM file")
        .arg(index_arg())
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(input_bam_arg())
}

fn print_subcommand_help(command_name: &str) -> Result<()> {
    let Some(mut command) = command_for_help(command_name) else {
        return Ok(());
    };
    write_command_help(&mut command)
}

fn write_command_help(command: &mut Command) -> Result<()> {
    let mut stderr = std::io::stderr();
    writeln!(&mut stderr).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    command
        .write_help(&mut stderr)
        .map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    writeln!(&mut stderr).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
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

fn threads_arg() -> Arg {
    Arg::new(ARG_THREADS)
        .short('@')
        .long("threads")
        .value_name("INT")
        .help("Number of htslib threads")
        .default_value("1")
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

fn threads(matches: &ArgMatches) -> Result<usize> {
    let threads = required_string(matches, ARG_THREADS)?;
    let threads = threads
        .parse::<usize>()
        .map_err(|_| "[qbix] threads must be a positive integer".to_string())?;
    if threads == 0 {
        return Err("[qbix] threads must be a positive integer".to_string());
    }
    Ok(threads)
}

fn get_order(matches: &ArgMatches) -> GetOrder {
    if matches.get_flag(ARG_BAM_ORDER) {
        GetOrder::Bam
    } else {
        GetOrder::Query
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

fn optional_values(matches: &ArgMatches, name: &str) -> Vec<String> {
    matches
        .get_many::<String>(name)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn get_readnames(matches: &ArgMatches) -> Result<Vec<String>> {
    let mut readnames = optional_values(matches, ARG_READNAMES);
    if let Some(path) = matches.get_one::<String>(ARG_READNAMES_FILE) {
        readnames.extend(readnames_from_path(path)?);
    }
    if readnames.is_empty() {
        return Err("[qbix] missing required argument: readnames".to_string());
    }
    Ok(readnames)
}

fn readnames_from_path(path: &str) -> Result<Vec<String>> {
    if path == "-" {
        let stdin = std::io::stdin();
        return readnames_from_reader(stdin.lock());
    }

    let file = std::fs::File::open(path)
        .map_err(|e| format!("[qbix] could not open read names file {path}: {e}"))?;
    readnames_from_reader(std::io::BufReader::new(file))
}

fn readnames_from_reader<R>(reader: R) -> Result<Vec<String>>
where
    R: BufRead,
{
    let mut readnames = Vec::new();
    for line in reader.lines() {
        let readname = line.map_err(|e| format!("[qbix] could not read read names: {e}"))?;
        if !readname.is_empty() {
            readnames.push(readname);
        }
    }
    Ok(readnames)
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
                threads: 1,
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
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string(), "read2".to_string()],
                threads: 4,
                order: GetOrder::Query,
                output_format: OutputFormat::Sam,
                output_path: None,
            }
        );
    }

    #[test]
    fn parses_get_bam_order() {
        let action = parse_args(strings([
            "qbix",
            "get",
            "--bam-order",
            "reads.bam",
            "read1",
            "read2",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string(), "read2".to_string()],
                threads: 1,
                order: GetOrder::Bam,
                output_format: OutputFormat::Sam,
                output_path: None,
            }
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
            "read1",
        ]))
        .unwrap();

        assert_eq!(
            action,
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string()],
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Bam,
                output_path: Some("hits.bam".to_string()),
            }
        );
    }

    #[test]
    fn parses_get_bam_shortcut() {
        let action = parse_args(strings(["qbix", "get", "reads.bam", "-b", "read1"])).unwrap();

        assert_eq!(
            action,
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string()],
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Bam,
                output_path: None,
            }
        );
    }

    #[test]
    fn parses_get_readnames_file() {
        let path = std::env::temp_dir().join(format!(
            "qbix-readnames-{}-{}.txt",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, "read1\n\nread2\n").unwrap();

        let action = parse_args(vec![
            "qbix".to_string(),
            "get".to_string(),
            "reads.bam".to_string(),
            "-f".to_string(),
            path.to_str().unwrap().to_string(),
        ])
        .unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            action,
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string(), "read2".to_string()],
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Sam,
                output_path: None,
            }
        );
    }

    #[test]
    fn parses_get_readnames_from_positional_and_file() {
        let path = std::env::temp_dir().join(format!(
            "qbix-readnames-combined-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, "read2\n").unwrap();

        let action = parse_args(vec![
            "qbix".to_string(),
            "get".to_string(),
            "reads.bam".to_string(),
            "read1".to_string(),
            "-f".to_string(),
            path.to_str().unwrap().to_string(),
        ])
        .unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            action,
            Action::Get {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                readnames: vec!["read1".to_string(), "read2".to_string()],
                threads: 1,
                order: GetOrder::Query,
                output_format: OutputFormat::Sam,
                output_path: None,
            }
        );
    }

    #[test]
    fn rejects_zero_threads() {
        let err =
            parse_args(strings(["qbix", "index", "--threads", "0", "reads.bam"])).unwrap_err();
        assert!(err.contains("positive integer"));
    }

    #[test]
    fn parses_check_options() {
        let action = parse_args(strings(["qbix", "check", "-v", "-@", "2", "reads.bam"])).unwrap();

        assert_eq!(
            action,
            Action::Check {
                input_bam: "reads.bam".to_string(),
                input_index: None,
                threads: 2,
                verbose: true,
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
        assert!(help.contains("Validate a QBI index against its BAM file"));
    }

    fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_string).collect()
    }
}
