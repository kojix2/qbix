use crate::commands::{self, GetOrder};
use crate::error::Result;
use crate::VERSION;
use clap::{error::ErrorKind, Arg, ArgAction, Command};
use std::io::Write;

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
        Action::Test {
            input_bam,
            input_index,
            threads,
            verbose,
        } => commands::test_index(&input_bam, input_index.as_deref(), threads, verbose),
        Action::Version => {
            println!("{VERSION}");
            Ok(())
        }
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
    Test {
        input_bam: String,
        input_index: Option<String>,
        threads: usize,
        verbose: bool,
    },
    Version,
    HelpDisplayed,
}

fn parse_args<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if args.len() == 1 {
        let mut app = app();
        let mut stderr = std::io::stderr();
        app.write_help(&mut stderr)
            .map_err(|e| format!("[qbix] could not write help text: {e}"))?;
        writeln!(&mut stderr).map_err(|e| format!("[qbix] could not write help text: {e}"))?;
        return Err("[qbix] no subcommand provided".to_string());
    }

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
        Err(err) => return Err(err.to_string()),
    };

    match matches.subcommand() {
        Some(("index", matches)) => Ok(Action::Index {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            output_index: optional_string(matches, "index"),
            verbose: matches.get_flag("verbose"),
            threads: threads(matches)?,
        }),
        Some(("get", matches)) => Ok(Action::Get {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            input_index: optional_string(matches, "index"),
            readnames: values(matches, "readnames")?,
            threads: threads(matches)?,
            order: get_order(matches),
        }),
        Some(("show", matches)) => Ok(Action::Show {
            input_index: required_string(matches, "input_index")?.to_string(),
        }),
        Some(("test", matches)) => Ok(Action::Test {
            input_bam: required_string(matches, "input_bam")?.to_string(),
            input_index: optional_string(matches, "index"),
            threads: threads(matches)?,
            verbose: matches.get_flag("verbose"),
        }),
        Some(("version", _)) => Ok(Action::Version),
        _ => Err("[qbix] usage qbix <subprogram> [...]".to_string()),
    }
}

fn app() -> Command {
    Command::new("qbix")
        .disable_version_flag(true)
        .subcommand_required(true)
        .subcommand(index_command())
        .subcommand(get_command())
        .subcommand(show_command())
        .subcommand(test_command())
        .subcommand(Command::new("version"))
}

fn index_command() -> Command {
    Command::new("index")
        .arg(index_arg("index_filename.qbi"))
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(Arg::new("input_bam").required(true))
}

fn get_command() -> Command {
    Command::new("get")
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
        .arg(Arg::new("input_bam").required(true))
        .arg(Arg::new("readnames").required(true).num_args(1..))
}

fn show_command() -> Command {
    Command::new("show").arg(Arg::new("input_index").required(true))
}

fn test_command() -> Command {
    Command::new("test")
        .arg(index_arg("index_filename.qbi"))
        .arg(threads_arg())
        .arg(verbose_arg())
        .arg(Arg::new("input_bam").required(true))
}

fn index_arg(value_name: &'static str) -> Arg {
    Arg::new("index")
        .short('i')
        .long("index")
        .value_name(value_name)
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
    fn parses_test_options() {
        let action = parse_args(strings(["qbix", "test", "-v", "-@", "2", "reads.bam"])).unwrap();

        assert_eq!(
            action,
            Action::Test {
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

    fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_string).collect()
    }
}
