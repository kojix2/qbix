mod common;

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use common::{write_unmapped_bam, TempDir};

#[test]
fn indexes_gets_shows_and_checks_a_synthetic_bam() {
    let temp = TempDir::new("e2e");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_b", "read_a", "read_a", "read_c"]);

    assert_success(Command::new(qbix()).args(["index", bam]));
    assert_success(Command::new(qbix()).args(["check", bam]));

    let get = Command::new(qbix())
        .args(["get", bam, "read_a", "read_c"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    let get_stdout = String::from_utf8(get.stdout).unwrap();
    let read_names: Vec<_> = get_stdout
        .lines()
        .map(|line| line.split('\t').next().unwrap())
        .collect();
    assert_eq!(read_names, ["read_a", "read_a", "read_c"]);

    let index = format!("{bam}.qbi");
    let show = Command::new(qbix())
        .args(["show", &index])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_stdout = String::from_utf8(show.stdout).unwrap();
    let rows: Vec<_> = show_stdout.lines().collect();
    assert_eq!(rows.len(), 4);
    for row in rows {
        let fields: Vec<_> = row.split('\t').collect();
        assert_eq!(fields.len(), 2);
        assert!(fields[0].parse::<u64>().is_ok());
        assert!(fields[1].parse::<i64>().is_ok());
    }
}

#[test]
fn get_can_emit_records_in_bam_order() {
    let temp = TempDir::new("bam-order");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_b", "read_a", "read_a", "read_c"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let query_order = Command::new(qbix())
        .args(["get", "--query-order", bam, "read_a", "read_b"])
        .output()
        .unwrap();
    assert!(
        query_order.status.success(),
        "{}",
        String::from_utf8_lossy(&query_order.stderr)
    );
    assert_eq!(
        first_fields(&query_order.stdout),
        ["read_a", "read_a", "read_b"]
    );

    let bam_order = Command::new(qbix())
        .args(["get", "--bam-order", bam, "read_a", "read_b"])
        .output()
        .unwrap();
    assert!(
        bam_order.status.success(),
        "{}",
        String::from_utf8_lossy(&bam_order.stderr)
    );
    assert_eq!(
        first_fields(&bam_order.stdout),
        ["read_b", "read_a", "read_a"]
    );
}

#[test]
fn get_can_read_names_from_file() {
    let temp = TempDir::new("readnames-file");
    let bam = temp.path().join("reads.bam");
    let names = temp.path().join("names.txt");
    let bam = bam.to_str().unwrap();
    let names = names.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b", "read_c"]);
    fs::write(names, "read_c\nread_a\n").unwrap();

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args(["get", bam, "-f", names])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_c", "read_a"]);
}

#[test]
fn get_can_read_names_from_stdin() {
    let temp = TempDir::new("readnames-stdin");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b", "read_c"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let mut child = Command::new(qbix())
        .args(["get", bam, "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"read_b\nread_a\n")
        .unwrap();
    let get = child.wait_with_output().unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_b", "read_a"]);
}

#[test]
fn supports_explicit_index_path() {
    let temp = TempDir::new("explicit-index");
    let bam = temp.path().join("reads.bam");
    let index = temp.path().join("custom.qbi");
    let bam = bam.to_str().unwrap();
    let index = index.to_str().unwrap();
    write_unmapped_bam(bam, &["read_x", "read_y"]);

    assert_success(Command::new(qbix()).args(["index", "-i", index, bam]));

    let get = Command::new(qbix())
        .args(["get", "-i", index, bam, "read_y"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_y"]);
}

#[test]
fn missing_readname_returns_empty_sam() {
    let temp = TempDir::new("missing-read");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a"]);
    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args(["get", bam, "not_present"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(get.stdout.is_empty());
}

#[test]
fn empty_bam_indexes_and_queries_cleanly() {
    let temp = TempDir::new("empty-bam");
    let bam = temp.path().join("empty.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &[]);

    assert_success(Command::new(qbix()).args(["index", bam]));
    assert_success(Command::new(qbix()).args(["check", bam]));

    let get = Command::new(qbix())
        .args(["get", bam, "anything"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(get.stdout.is_empty());

    let index = format!("{bam}.qbi");
    let show = Command::new(qbix())
        .args(["show", &index])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    assert!(show.stdout.is_empty());
}

#[test]
fn rejects_unsupported_index_format() {
    let temp = TempDir::new("corrupt-index");
    let index = temp.path().join("bad.qbi");
    let mut file = fs::File::create(&index).unwrap();
    let mut bad_index = [0u8; 48];
    bad_index[..4].copy_from_slice(b"NOPE");
    file.write_all(&bad_index).unwrap();

    let output = Command::new(qbix())
        .args(["show", index.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported index format"));
}

#[test]
fn no_arguments_prints_help_to_stderr_and_fails() {
    let output = Command::new(qbix()).output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with('\n'));
    assert!(stderr.contains("Program: qbix"));
    assert!(stderr.contains("Version:"));
    assert!(stderr.contains("Source:"));
    assert!(stderr.contains("Usage:   qbix <command> [options]"));
    assert!(stderr.contains("no subcommand provided"));
    assert!(stderr.contains("[qbix] no subcommand provided"));
}

#[test]
fn subcommand_without_required_arguments_prints_help_to_stderr_and_fails() {
    let output = Command::new(qbix()).arg("check").output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage:"));
    assert!(stderr.contains("check"));
    assert!(stderr.contains("<input.bam>"));
    assert!(stderr.contains("required"));
    assert!(stderr.lines().any(|line| line.starts_with("[qbix]")));
    assert!(stderr.lines().any(|line| line.starts_with("Usage:")));
}

#[test]
fn subcommand_help_starts_with_blank_line() {
    let output = Command::new(qbix())
        .args(["index", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with('\n'));
    assert!(stderr.contains("Build a QNAME index for a BAM file"));
}

#[test]
fn accepts_threads_option_for_htslib_backed_commands() {
    let temp = TempDir::new("threads");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", "-@", "2", bam]));
    assert_success(Command::new(qbix()).args(["check", "--threads", "2", bam]));

    let get = Command::new(qbix())
        .args(["get", "-@", "2", bam, "read_b"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_b"]);
}

fn qbix() -> &'static str {
    env!("CARGO_BIN_EXE_qbix")
}

fn assert_success(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn first_fields(output: &[u8]) -> Vec<&str> {
    let output = std::str::from_utf8(output).unwrap();
    output
        .lines()
        .map(|line| line.split('\t').next().unwrap())
        .collect()
}
