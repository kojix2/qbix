mod common;

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

use common::{write_unmapped_bam, TempDir};

#[test]
fn indexes_gets_shows_and_checks_a_synthetic_bam() {
    let temp = TempDir::new("e2e");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_b", "read_a", "read_a", "read_c"]);

    assert_success(Command::new(qbix()).args(["index", bam]));
    let quick_check = Command::new(qbix()).args(["check", bam]).output().unwrap();
    assert!(
        quick_check.status.success(),
        "{}",
        String::from_utf8_lossy(&quick_check.stderr)
    );
    assert!(String::from_utf8_lossy(&quick_check.stderr).contains("ok (quick, 4 records)"));
    assert_success(Command::new(qbix()).args(["check", "--quick", bam]));
    let full_check = Command::new(qbix())
        .args(["check", "--full", bam])
        .output()
        .unwrap();
    assert!(
        full_check.status.success(),
        "{}",
        String::from_utf8_lossy(&full_check.stderr)
    );
    assert!(String::from_utf8_lossy(&full_check.stderr).contains("ok (full, 4 records)"));

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

    let stats = Command::new(qbix()).args(["stats", bam]).output().unwrap();
    assert!(
        stats.status.success(),
        "{}",
        String::from_utf8_lossy(&stats.stderr)
    );
    let stats_stdout = String::from_utf8(stats.stdout).unwrap();
    assert!(stats_stdout.contains("Records:\t4"));
    assert!(stats_stdout.contains("Distinct read-name hashes:\t3"));
    assert!(stats_stdout.contains("  1 (singletons):\t2 (66.7%)"));
    assert!(stats_stdout.contains("  2 (pairs):\t1 (33.3%)"));
    assert!(stats_stdout.contains("  3+ (multi/suppl.):\t0 (0.0%)"));
    assert!(stats_stdout.contains("  max:\t2"));
    assert!(stats_stdout.contains("  mean:\t1.33"));
    assert!(stats_stdout.contains("Index metadata:"));
    assert!(stats_stdout.contains("  BAM:\t"));
    assert!(stats_stdout.contains("  Index:\t"));

    let json_stats = Command::new(qbix())
        .args(["stat", "--json", bam])
        .output()
        .unwrap();
    assert!(
        json_stats.status.success(),
        "{}",
        String::from_utf8_lossy(&json_stats.stderr)
    );
    let json_stdout = String::from_utf8(json_stats.stdout).unwrap();
    assert!(json_stdout.contains("\"format\": \"QBI1\""));
    assert!(json_stdout.contains("\"qbi2_radix_bits\": null"));
    assert!(json_stdout.contains("\"records\": 4"));
    assert!(json_stdout.contains("\"distinct_qname_hashes\": 3"));
    assert!(json_stdout.contains("\"singletons\": 2"));
    assert!(json_stdout.contains("\"pairs\": 1"));
    assert!(json_stdout.contains("\"multi_or_supplementary\": 0"));
    assert!(json_stdout.contains("\"max\": 2"));
    assert!(json_stdout.contains("\"qbi1\": 112"));
    assert!(json_stdout.contains("\"qbi2_p8\": 2261"));
    assert!(json_stdout.contains("\"qbi2_p12\": 32981"));
    assert!(json_stdout.contains("\"qbi2_p16\": 524498"));
    assert!(json_stdout.contains("\"smallest_qbi2_radix_bits\": 8"));
}

#[test]
fn qbi2_matches_qbi1_cli_behavior() {
    let temp = TempDir::new("qbi2-e2e");
    let bam = temp.path().join("reads.bam");
    let qbi1 = temp.path().join("reads.qbi1");
    let qbi2 = temp.path().join("reads.qbi2");
    let qbi2_p12 = temp.path().join("reads-p12.qbi2");
    let qbi2_p16 = temp.path().join("reads-p16.qbi2");
    let bam = bam.to_str().unwrap();
    let qbi1 = qbi1.to_str().unwrap();
    let qbi2 = qbi2.to_str().unwrap();
    let qbi2_p12 = qbi2_p12.to_str().unwrap();
    let qbi2_p16 = qbi2_p16.to_str().unwrap();
    write_unmapped_bam(bam, &["read_b", "read_a", "read_a", "read_c"]);

    assert_success(Command::new(qbix()).args(["index", "--index-format", "qbi1", "-i", qbi1, bam]));
    assert_success(Command::new(qbix()).args(["index", "--index-format", "qbi2", "-i", qbi2, bam]));
    assert_success(Command::new(qbix()).args([
        "index",
        "--index-format",
        "qbi2",
        "--qbi2-radix-bits",
        "12",
        "-i",
        qbi2_p12,
        bam,
    ]));
    assert_success(Command::new(qbix()).args([
        "index",
        "--index-format",
        "qbi2",
        "--qbi2-radix-bits",
        "16",
        "-i",
        qbi2_p16,
        bam,
    ]));
    assert_eq!(fs::metadata(qbi1).unwrap().len(), 112);
    assert_eq!(fs::metadata(qbi2).unwrap().len(), 2_261);
    let p8_bytes = fs::read(qbi2).unwrap();
    assert_eq!(&p8_bytes[6..8], &[0, 0]);
    assert_eq!(p8_bytes[8], 8);
    assert_eq!(p8_bytes[10], 3);
    assert_eq!(fs::metadata(qbi2_p12).unwrap().len(), 32_981);
    assert_eq!(fs::read(qbi2_p12).unwrap()[8], 12);
    assert_eq!(fs::metadata(qbi2_p16).unwrap().len(), 524_498);
    assert_eq!(fs::read(qbi2_p16).unwrap()[8], 16);

    let qbi1_show = Command::new(qbix()).args(["show", qbi1]).output().unwrap();
    let qbi2_show = Command::new(qbix()).args(["show", qbi2]).output().unwrap();
    assert!(qbi1_show.status.success());
    assert!(qbi2_show.status.success());
    assert_eq!(qbi2_show.stdout, qbi1_show.stdout);

    for index in [qbi1, qbi2, qbi2_p12, qbi2_p16] {
        assert_success(Command::new(qbix()).args(["check", "--full", "-i", index, bam]));
    }
    let get = Command::new(qbix())
        .args(["get", "-i", qbi2, bam, "read_a", "read_c"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_a", "read_a", "read_c"]);

    let stats = Command::new(qbix())
        .args(["stats", "--json", "-i", qbi2, bam])
        .output()
        .unwrap();
    assert!(stats.status.success());
    let stats = String::from_utf8_lossy(&stats.stdout);
    assert!(stats.contains("\"format\": \"QBI2\""));
    assert!(stats.contains("\"qbi2_radix_bits\": 8"));
}

#[test]
fn qbi2_full_check_performs_deferred_directory_validation() {
    let temp = TempDir::new("qbi2-deferred-check");
    let bam = temp.path().join("reads.bam");
    let index = temp.path().join("reads.qbi");
    let names = (0..600)
        .map(|index| format!("read_{index:04}"))
        .collect::<Vec<_>>();
    let name_refs = names.iter().map(String::as_str).collect::<Vec<_>>();
    write_unmapped_bam(bam.to_str().unwrap(), &name_refs);
    assert_success(Command::new(qbix()).args([
        "index",
        "--index-format",
        "qbi2",
        "-i",
        index.to_str().unwrap(),
        bam.to_str().unwrap(),
    ]));

    let bytes = fs::read(&index).unwrap();
    let rank_offset = u64::from_le_bytes(bytes[80..88].try_into().unwrap());
    let mut file = fs::OpenOptions::new().write(true).open(&index).unwrap();
    file.seek(SeekFrom::Start(rank_offset + 8)).unwrap();
    file.write_all(&999u64.to_le_bytes()).unwrap();
    drop(file);

    assert_success(Command::new(qbix()).args([
        "check",
        "--quick",
        "-i",
        index.to_str().unwrap(),
        bam.to_str().unwrap(),
    ]));
    let full = Command::new(qbix())
        .args([
            "check",
            "--full",
            "-i",
            index.to_str().unwrap(),
            bam.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!full.status.success());
    assert!(String::from_utf8_lossy(&full.stderr).contains("rank directory"));
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
fn get_can_process_duplicate_query_names_only_once() {
    let temp = TempDir::new("unique-query-names");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let repeated = Command::new(qbix())
        .args(["get", bam, "read_a", "read_a"])
        .output()
        .unwrap();
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert_eq!(
        first_fields(&repeated.stdout),
        ["read_a", "read_a", "read_a", "read_a"]
    );

    let unique = Command::new(qbix())
        .args(["get", "--bam-order", "--unique", bam, "read_a", "read_a"])
        .output()
        .unwrap();
    assert!(
        unique.status.success(),
        "{}",
        String::from_utf8_lossy(&unique.stderr)
    );
    assert_eq!(first_fields(&unique.stdout), ["read_a", "read_a"]);
}

#[test]
fn get_can_report_missing_query_names() {
    let temp = TempDir::new("missing-query-names");
    let bam = temp.path().join("reads.bam");
    let missing = temp.path().join("missing.txt");
    let unique_missing = temp.path().join("unique-missing.txt");
    let no_missing = temp.path().join("no-missing.txt");
    let bam = bam.to_str().unwrap();
    let missing = missing.to_str().unwrap();
    let unique_missing = unique_missing.to_str().unwrap();
    let no_missing = no_missing.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args([
            "get",
            "--missing",
            missing,
            bam,
            "read_a",
            "not_present",
            "not_present",
        ])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_a", "read_a"]);
    assert_eq!(
        fs::read_to_string(missing).unwrap(),
        "not_present\nnot_present\n"
    );

    let unique = Command::new(qbix())
        .args([
            "get",
            "--bam-order",
            "--unique",
            "--missing",
            unique_missing,
            bam,
            "not_present",
            "not_present",
        ])
        .output()
        .unwrap();
    assert!(
        unique.status.success(),
        "{}",
        String::from_utf8_lossy(&unique.stderr)
    );
    assert!(unique.stdout.is_empty());
    assert_eq!(fs::read_to_string(unique_missing).unwrap(), "not_present\n");

    let all_found = Command::new(qbix())
        .args(["get", "--missing", no_missing, bam, "read_b"])
        .output()
        .unwrap();
    assert!(
        all_found.status.success(),
        "{}",
        String::from_utf8_lossy(&all_found.stderr)
    );
    assert_eq!(first_fields(&all_found.stdout), ["read_b"]);
    assert!(fs::read(no_missing).unwrap().is_empty());
}

#[test]
fn get_reports_missing_names_in_query_order_with_bam_order_output() {
    let temp = TempDir::new("missing-with-bam-order");
    let bam = temp.path().join("reads.bam");
    let missing = temp.path().join("missing.txt");
    let bam = bam.to_str().unwrap();
    let missing = missing.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args([
            "get",
            "--bam-order",
            "--missing",
            missing,
            bam,
            "missing_b",
            "read_b",
            "missing_a",
            "read_a",
        ])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["read_a", "read_b"]);
    assert_eq!(
        fs::read_to_string(missing).unwrap(),
        "missing_b\nmissing_a\n"
    );
}

#[test]
fn get_can_read_crlf_names_from_file() {
    let temp = TempDir::new("readnames-file-crlf");
    let bam = temp.path().join("reads.bam");
    let names = temp.path().join("names.txt");
    let bam = bam.to_str().unwrap();
    let names = names.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b", "read_c"]);
    fs::write(names, b"read_c\r\nread_a\r\n").unwrap();

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
        .args(["get", bam, "-f", "-", "--query-order"])
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
fn get_can_include_the_source_header_in_sam_output() {
    let temp = TempDir::new("sam-header");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args(["get", "--with-header", bam, "read_a"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(first_fields(&get.stdout), ["@HD", "@SQ", "read_a"]);
}

#[test]
fn query_order_processes_stdin_before_a_later_read_error() {
    let temp = TempDir::new("streaming-stdin");
    let bam = temp.path().join("reads.bam");
    let output = temp.path().join("hits.sam");
    let bam = bam.to_str().unwrap();
    let output = output.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let mut child = Command::new(qbix())
        .args(["get", "--query-order", "-f", "-", "-o", output, bam])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"read_a\n\xff\n")
        .unwrap();
    let get = child.wait_with_output().unwrap();

    assert!(!get.status.success());
    assert!(String::from_utf8_lossy(&get.stderr).contains("could not read read names"));
    assert_eq!(
        first_fields(&fs::read(output).unwrap()),
        ["read_a"],
        "the first query must be processed before the later stdin error"
    );
}

#[test]
#[cfg(not(feature = "biosyntax"))]
fn get_omits_color_option_without_biosyntax_feature() {
    let temp = TempDir::new("color-disabled");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let help = Command::new(qbix())
        .args(["get", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(!String::from_utf8_lossy(&help.stdout).contains("--color"));

    let get = Command::new(qbix())
        .args(["get", "--color", "always", bam, "read_a"])
        .output()
        .unwrap();
    assert!(!get.status.success());
    assert!(String::from_utf8_lossy(&get.stderr).contains("unexpected argument"));
}

#[test]
#[cfg(feature = "biosyntax")]
fn get_can_force_colored_sam_output_with_biosyntax_feature() {
    let temp = TempDir::new("color-enabled");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a"]);

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args(["get", "--color", "always", "--with-header", bam, "read_a"])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    let stdout = String::from_utf8_lossy(&get.stdout);
    let first_line = stdout.lines().next().unwrap();
    assert!(first_line.contains("\x1b["));
    assert!(first_line.contains("@HD"));
}

#[test]
fn get_can_write_bam_output_to_path() {
    let temp = TempDir::new("bam-output");
    let bam = temp.path().join("reads.bam");
    let names = temp.path().join("names.txt");
    let hits = temp.path().join("hits.bam");
    let bam = bam.to_str().unwrap();
    let names = names.to_str().unwrap();
    let hits = hits.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b", "read_c"]);
    fs::write(names, "read_c\nread_a\n").unwrap();

    assert_success(Command::new(qbix()).args(["index", bam]));

    let get = Command::new(qbix())
        .args(["get", bam, "-f", names, "-Ob", "-o", hits])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(get.stdout.is_empty());

    assert_success(Command::new(qbix()).args(["index", hits]));
    let verify = Command::new(qbix())
        .args(["get", hits, "read_a", "read_c"])
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert_eq!(first_fields(&verify.stdout), ["read_a", "read_c"]);
}

#[test]
fn get_rejects_output_paths_that_would_overwrite_inputs() {
    let temp = TempDir::new("output-path-conflicts");
    let bam = temp.path().join("reads.bam");
    let names = temp.path().join("names.txt");
    let shared_output = temp.path().join("shared.txt");
    let bam = bam.to_str().unwrap();
    let names = names.to_str().unwrap();
    let index = format!("{bam}.qbi");
    let shared_output = shared_output.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a"]);
    fs::write(names, "read_a\n").unwrap();

    assert_success(Command::new(qbix()).args(["index", bam]));
    let bam_before = fs::read(bam).unwrap();
    let index_before = fs::read(&index).unwrap();
    let names_before = fs::read(names).unwrap();
    let cases = [
        (
            vec!["index", "--index", bam, bam],
            "output index must not overwrite the input BAM",
        ),
        (
            vec!["get", "--output", bam, bam, "read_a"],
            "must not overwrite the input BAM",
        ),
        (
            vec!["get", "--missing", bam, bam, "read_a"],
            "must not overwrite the input BAM",
        ),
        (
            vec!["get", "--output", &index, bam, "read_a"],
            "must not overwrite the input index",
        ),
        (
            vec!["get", "--missing", &index, bam, "read_a"],
            "must not overwrite the input index",
        ),
        (
            vec![
                "get",
                "--output",
                shared_output,
                "--missing",
                shared_output,
                bam,
                "read_a",
            ],
            "must use different paths",
        ),
        (
            vec!["get", "--file", names, "--output", names, bam],
            "must not overwrite the read-name input",
        ),
        (
            vec!["get", "--file", names, "--missing", names, bam],
            "must not overwrite the read-name input",
        ),
    ];

    for (args, expected_error) in cases {
        let get = Command::new(qbix()).args(args).output().unwrap();
        assert!(!get.status.success());
        assert!(
            String::from_utf8_lossy(&get.stderr).contains(expected_error),
            "{}",
            String::from_utf8_lossy(&get.stderr)
        );
        assert_eq!(fs::read(bam).unwrap(), bam_before);
        assert_eq!(fs::read(&index).unwrap(), index_before);
        assert_eq!(fs::read(names).unwrap(), names_before);
    }
    assert!(!std::path::Path::new(shared_output).exists());
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
fn explicit_help_prints_to_stdout() {
    let output = Command::new(qbix())
        .args(["index", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with('\n'));
    assert!(stdout.contains("Build a QNAME index for a BAM file"));
}

#[test]
fn top_level_help_prints_to_stdout() {
    let output = Command::new(qbix()).arg("--help").output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with('\n'));
    assert!(stdout.contains("Program: qbix"));
    assert!(stdout.contains("Usage:   qbix <command> [options]"));
}

#[test]
fn accepts_bgzf_threads_option_for_htslib_backed_commands() {
    let temp = TempDir::new("threads");
    let bam = temp.path().join("reads.bam");
    let bam = bam.to_str().unwrap();
    write_unmapped_bam(bam, &["read_a", "read_b"]);

    assert_success(Command::new(qbix()).args(["index", "-@", "2", bam]));
    assert_success(Command::new(qbix()).args(["check", "--bgzf-threads", "2", bam]));

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
