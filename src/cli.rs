use crate::commands::{self, GetOrder};
use crate::error::Result;
use crate::VERSION;
use clap::{error::ErrorKind, Arg, ArgAction, Command};
use std::io::Write;

const COMMAND_INDEX: &str = "index";
const COMMAND_GET: &str = "get";
const COMMAND_SHOW: &str = "show";
const COMMAND_CHECK: &str = "check";

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

fn parse_args<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
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
            return Err(err.to_string());
        }
        Err(err) => return Err(err.to_string()),
    };

    match matches.subcommand() {
        Some((COMMAND_INDEX, matches)) => Ok(Action::Index {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            output_index: optional_string(matches, "index"),
            verbose: matches.get_flag("verbose"),
            threads: threads(matches)?,
        }),
        Some((COMMAND_GET, matches)) => Ok(Action::Get {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            input_index: optional_string(matches, "index"),
            readnames: values(matches, "readnames")?,
            threads: threads(matches)?,
            order: get_order(matches),
        }),
        Some((COMMAND_SHOW, matches)) => Ok(Action::Show {
            input_index: required_string(matches, "input_index")?.to_string(),
        }),
        Some((COMMAND_CHECK, matches)) => Ok(Action::Check {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            input_index: optional_string(matches, "index"),
            threads: threads(matches)?,
            verbose: matches.get_flag("verbose"),
        }),
        _ => Err("[qbix] usage qbix <COMMAND> [...]".to_string()),
    }
}

fn app() -> Command {
    Command::new("qbix")
        .about("Retrieve BAM records by read name using a QBI index")
        .version(VERSION)
        .disable_help_subcommand(true)
        .subcommand_required(true)
        .subcommand(index_command())
        .subcommand(get_command())
        .subcommand(show_command())
        .subcommand(check_command())
}

fn index_command() -> Command {
    Command::new(COMMAND_INDEX)
        .about("Build a QBI index for a BAM file")
        .arg(index_arg("index_filename.qbi"))
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(input_bam_arg())
}

fn get_command() -> Command {
    Command::new(COMMAND_GET)
        .about("Fetch BAM records by read name")
        .arg(index_arg("index_filename.qbi"))
        .arg(threads_arg())
        .arg(
            Arg::new("bam_order")
                .long("bam-order")
                .action(ArgAction::SetTrue)
                .conflicts_with("query_order"),
        )
        .arg(
            Arg::new("query_order")
                .long("query-order")
                .action(ArgAction::SetTrue)
                .conflicts_with("bam_order"),
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
        .arg(index_arg("index_filename.qbi"))
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(input_bam_arg())
}

fn print_subcommand_help(command_name: &str) -> Result<()> {
    let Some(mut command) = subcommand(command_name) else {
        return Ok(());
    };
    command = command.bin_name(format!("qbix {command_name}"));
    write_command_help(&mut command)
}

fn write_command_help(command: &mut Command) -> Result<()> {
    let mut stderr = std::io::stderr();
    command
        .write_help(&mut stderr)
        .map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    writeln!(&mut stderr).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
    Ok(())
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

fn index_arg(value_name: &'static str) -> Arg {
    Arg::new("index")
        .short('i')
        .long("index")
        .value_name(value_name)
}

fn input_bam_arg() -> Arg {
    Arg::new("input_bam").value_name("input.bam").required(true)
}

fn input_index_arg() -> Arg {
    Arg::new("input_index")
        .value_name("input.qbi")
        .required(true)
}

fn readnames_arg() -> Arg {
    Arg::new("readnames")
        .value_name("readname")
        .required(true)
        .num_args(1..)
}

fn threads_arg() -> Arg {
    Arg::new("threads")
        .short('@')
        .long("threads")
        .value_name("INT")
        .default_value("1")
}

fn verbose_arg() -> Arg {
    Arg::new("verbose")
        .short('v')
        .long("verbose")
        .action(ArgAction::SetTrue)
}

fn required_string<'a>(matches: &'a clap::ArgMatches, name: &str) -> Result<&'a str> {
    matches
        .get_one::<String>(name)
        .map(String::as_str)
        .ok_or_else(|| format!("[qbix] missing required argument: {name}"))
}

fn optional_string(matches: &clap::ArgMatches, name: &str) -> Option<String> {
    matches.get_one::<String>(name).cloned()
}

fn threads(matches: &clap::ArgMatches) -> Result<usize> {
    let threads = required_string(matches, "threads")?;
    let threads = threads
        .parse::<usize>()
        .map_err(|_| "[qbix] threads must be a positive integer".to_string())?;
    if threads == 0 {
        return Err("[qbix] threads must be a positive integer".to_string());
    }
    Ok(threads)
}

fn get_order(matches: &clap::ArgMatches) -> GetOrder {
    if matches.get_flag("bam_order") {
        GetOrder::Bam
    } else {
        GetOrder::Query
    }
}

fn values(matches: &clap::ArgMatches, name: &str) -> Result<Vec<String>> {
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

        assert!(help.contains("Build a QBI index for a BAM file"));
        assert!(help.contains("Fetch BAM records by read name"));
        assert!(help.contains("Print raw QBI index rows"));
        assert!(help.contains("Validate a QBI index against its BAM file"));
    }

    fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_string).collect()
    }
}
