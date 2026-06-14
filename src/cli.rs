use crate::commands::{self, GetOrder};
use crate::error::Result;
use crate::VERSION;
use clap::{error::ErrorKind, Arg, ArgAction, ArgMatches, Command};
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
        } => commands::get_records(
            &input_bam,
            input_index.as_deref(),
            &readnames,
            threads,
            order,
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
            readnames: values(matches, ARG_READNAMES)?,
            threads: threads(matches)?,
            order: get_order(matches),
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
        .required(true)
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

fn values(matches: &ArgMatches, name: &str) -> Result<Vec<String>> {
    matches
        .get_many::<String>(name)
        .map(|values| values.cloned().collect())
        .ok_or_else(|| format!("[qbix] missing required argument: {name}"))
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
