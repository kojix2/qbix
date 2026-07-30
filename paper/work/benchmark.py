#!/usr/bin/env python3
"""Reproducible quick benchmarks for qbix."""

from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import heapq
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import statistics
import subprocess
import time
from typing import BinaryIO, Sequence


WORK_DIR = Path(__file__).resolve().parent
REPO_DIR = WORK_DIR.parent.parent
DEFAULT_OUTPUT_ROOT = WORK_DIR / "output"
TOOLS_DIR = WORK_DIR / "tools"
SEED = 20260730
ID_PATTERN = re.compile(r"^[A-Za-z0-9._-]+$")
PLOT_TIME_FLOOR_SECONDS = 0.005
HEADER_INDEX_RUNS = [
    "run_id", "dataset", "tool", "index_format", "radix_bits",
    "replicate", "cache_condition",
    "bgzf_threads", "sort_threads", "memory", "bucket_bits",
    "elapsed_s", "user_s", "sys_s", "max_rss_kb", "exit_status",
    "temp_bytes", "index_bytes",
]
HEADER_QUERY_RUNS = [
    "run_id", "dataset", "tool", "index_format", "radix_bits", "mode",
    "query_type", "query_count", "replicate", "cache_condition", "elapsed_s",
    "user_s", "sys_s", "max_rss_kb", "exit_status", "expected_output_records",
    "query_sha256",
]


@dataclass(frozen=True)
class QbixLayout:
    name: str
    index_format: str
    radix_bits: int | None = None


QBIX_LAYOUTS = {
    "qbi1": QbixLayout("qbi1", "qbi1"),
    "qbi2-p8": QbixLayout("qbi2-p8", "qbi2", 8),
    "qbi2-p12": QbixLayout("qbi2-p12", "qbi2", 12),
    "qbi2-p16": QbixLayout("qbi2-p16", "qbi2", 16),
}


@dataclass(frozen=True)
class Timing:
    elapsed_s: float
    user_s: float
    sys_s: float
    max_rss_kb: int
    exit_status: int
    temp_bytes: int = 0


class Benchmark:
    def __init__(self, args: argparse.Namespace) -> None:
        self.bam = args.bam.resolve()
        if not self.bam.is_file():
            raise SystemExit(f"error: BAM file not found: {self.bam}")
        self.run_id = validate_id(args.run_id, "run ID")
        self.dataset_id = validate_id(args.dataset_id, "dataset ID")
        self.platform = args.platform
        self.source = args.source
        self.region = args.region
        self.storage = args.storage
        self.profile = args.profile
        self.replicates = 3 if self.profile == "quick" else 5
        self.index_replicates = 3
        self.requested_query_sizes = (
            (1, 100, 10_000)
            if self.profile == "quick"
            else (1, 10, 100, 1_000, 10_000)
        )
        layout_names = args.qbix_layouts or (
            ["qbi1"] if self.profile == "quick" else list(QBIX_LAYOUTS)
        )
        self.qbix_layouts = [QBIX_LAYOUTS[name] for name in layout_names]
        self.parameter_layout = QBIX_LAYOUTS[args.parameter_layout]
        self.tools = set(args.tools)
        self.output_root = args.output_root.resolve()
        self.run_dir = self.output_root / self.run_id
        self.out = self.run_dir / self.dataset_id
        self.queries = self.out / "queries"
        self.tmp = self.out / "tmp"
        self.qbi_paths = {
            layout.name: self.out / (
                "benchmark.qbi"
                if self.profile == "quick" and layout.name == "qbi1"
                else f"benchmark.{layout.name}.qbi"
            )
            for layout in self.qbix_layouts
        }
        # bri's `-i <path>` option triggers a 1-byte heap overflow in bri's own
        # generate_index_filename() (malloc(strlen(path)) with no room for the
        # NUL terminator), which aborts under glibc's _FORTIFY_SOURCE. Avoid
        # `-i` entirely and rely on bri's default "<bam>.bri" naming instead.
        self.bri_index = Path(f"{self.bam}.bri")
        self.atlantool_index = self.out / "benchmark.atlantool-index"
        for path in (self.out, self.queries, self.tmp):
            path.mkdir(parents=True, exist_ok=True)

        self.qbix = Path(
            os.environ.get("QBIX", REPO_DIR / "target" / "release" / "qbix")
        )
        self.external_qbix = "QBIX" in os.environ
        self.samtools = os.environ.get("SAMTOOLS", "samtools")
        self.time_bin = os.environ.get("TIME_BIN", "time")
        require_command(self.samtools)
        require_command(self.time_bin)
        require_command("sort")

        self.bri = Path(os.environ.get("BRI", TOOLS_DIR / "bri"))
        self.have_bri = self.bri.is_file()
        self.atlantool = Path(os.environ.get("ATLANTOOL", TOOLS_DIR / "atlantool-linux"))
        self.have_atlantool = self.atlantool.is_file()
        if "bri" in self.tools and not self.have_bri:
            print(f"note: bri not found at {self.bri}; skipping bri comparison "
                  f"(run setup_tools.sh or set BRI=)")
        if "atlantool" in self.tools and not self.have_atlantool:
            print(f"note: atlantool not found at {self.atlantool}; skipping atlantool "
                  f"comparison (run setup_tools.sh or set ATLANTOOL=)")

        self.ensure_qbix()
        self.ensure_manifest()

    def comparison_methods(self) -> list[str]:
        methods = [
            f"qbix:{layout.name}:{order}"
            for layout in self.qbix_layouts
            for order in ("query", "bam")
        ]
        if "samtools" in self.tools:
            methods.append("samtools")
        if "bri" in self.tools and self.have_bri:
            methods.append("bri")
        if "atlantool" in self.tools and self.have_atlantool:
            methods.append("atlantool")
        return methods

    def ensure_qbix(self) -> None:
        if not self.external_qbix:
            require_command("cargo")
            subprocess.run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "--locked",
                    "--manifest-path",
                    str(REPO_DIR / "Cargo.toml"),
                ],
                check=True,
            )
        if not self.qbix.is_file():
            raise SystemExit(f"error: qbix binary was not created: {self.qbix}")

    def ensure_manifest(self) -> None:
        manifest_path = self.out / "manifest.json"
        bam_stat = self.bam.stat()
        identity = {
            "run_id": self.run_id,
            "dataset_id": self.dataset_id,
            "bam_path": str(self.bam),
            "bam_bytes": bam_stat.st_size,
            "bam_mtime_ns": bam_stat.st_mtime_ns,
            "profile": self.profile,
            "qbix_layouts": [layout.name for layout in self.qbix_layouts],
            "platform": self.platform,
            "source": self.source,
            "region": self.region,
            "storage": self.storage,
            "parameter_layout": self.parameter_layout.name,
            "tools": sorted(self.tools),
        }
        if manifest_path.exists():
            old = json.loads(manifest_path.read_text())
            for key, value in identity.items():
                if old.get(key) != value:
                    raise SystemExit(
                        f"error: {manifest_path} belongs to different input data"
                    )
            current_qbix_hash = sha256_file(self.qbix)
            if old.get("qbix_sha256") != current_qbix_hash:
                raise SystemExit(
                    "error: qbix binary changed during this benchmark run; "
                    "start a new run ID"
                )
            return
        manifest = {
            **identity,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "seed": SEED,
            "replicates": self.replicates,
            "index_replicates": self.index_replicates,
            "requested_query_sizes": self.requested_query_sizes,
            "qbix_path": str(self.qbix.resolve()),
            "qbix_sha256": sha256_file(self.qbix),
            "git_commit": command_output(
                ["git", "-C", str(REPO_DIR), "rev-parse", "HEAD"]
            ).strip(),
            "git_status_porcelain": command_output(
                ["git", "-C", str(REPO_DIR), "status", "--short"]
            ).splitlines(),
            "cache_policy": "BAM read once; filesystem cache not explicitly cleared",
            "index_options": {
                "bgzf_threads": 1,
                "sort_threads": 1,
                "memory": "512M",
                "bucket_bits": 8,
            },
        }
        write_json(manifest_path, manifest)

    def log_command(self, command: Sequence[str | Path], label: str) -> None:
        record = {
            "time": datetime.now(timezone.utc).isoformat(),
            "label": label,
            "command": [str(value) for value in command],
            "shell": shlex.join(str(value) for value in command),
        }
        with (self.out / "commands.jsonl").open("a") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")

    def run(
        self,
        command: Sequence[str | Path],
        *,
        label: str,
        stdout: int | BinaryIO | None = None,
    ) -> subprocess.CompletedProcess[bytes]:
        self.log_command(command, label)
        return subprocess.run(
            [str(value) for value in command],
            stdout=stdout,
            check=True,
        )

    def measure(
        self,
        command: Sequence[str | Path],
        *,
        label: str,
        stdout: int | BinaryIO | None = None,
        temp_paths: Sequence[Path] = (),
    ) -> Timing:
        timing_path = self.tmp / f"{label}.time"
        wrapped = [
            self.time_bin,
            "-f",
            r"%e\t%U\t%S\t%M\t%x",
            "-o",
            timing_path,
            *command,
        ]
        self.log_command(command, label)
        process = subprocess.Popen(
            [str(value) for value in wrapped],
            stdout=stdout,
        )
        peak_temp_bytes = 0
        while process.poll() is None:
            peak_temp_bytes = max(peak_temp_bytes, paths_size(temp_paths))
            time.sleep(0.25)
        peak_temp_bytes = max(peak_temp_bytes, paths_size(temp_paths))
        values = timing_path.read_text().strip().split("\t")
        if len(values) != 5:
            raise SystemExit(f"error: invalid timing output: {timing_path}")
        timing = Timing(
            elapsed_s=float(values[0]),
            user_s=float(values[1]),
            sys_s=float(values[2]),
            max_rss_kb=int(values[3]),
            exit_status=int(values[4]),
            temp_bytes=peak_temp_bytes,
        )
        if process.returncode != 0:
            raise SystemExit(
                f"error: command failed with status {process.returncode}: "
                + shlex.join(str(value) for value in command)
            )
        return timing

    def preflight(self, max_index_s: float, max_scan_s: float) -> None:
        preflight_qbi = self.tmp / "preflight.qbi"
        preflight_tmp = self.tmp / "preflight-index"
        preflight_qbi.unlink(missing_ok=True)
        shutil.rmtree(preflight_tmp, ignore_errors=True)
        preflight_tmp.mkdir()
        index_timing = self.measure(
            self.qbix_index_command(preflight_qbi, preflight_tmp),
            label="preflight-qbix-index",
            stdout=subprocess.DEVNULL,
        )
        scan_timing = self.measure(
            [self.samtools, "view", "-@", "1", "-c", self.bam],
            label="preflight-samtools-scan",
            stdout=subprocess.DEVNULL,
        )
        preflight_qbi.unlink(missing_ok=True)
        shutil.rmtree(preflight_tmp, ignore_errors=True)
        rows = [
            ("qbix-index", index_timing.elapsed_s, max_index_s),
            ("samtools-scan", scan_timing.elapsed_s, max_scan_s),
        ]
        write_tsv(
            self.out / "preflight.tsv",
            ["operation", "elapsed_s", "limit_s", "within_limit"],
            [(name, elapsed, limit, elapsed <= limit) for name, elapsed, limit in rows],
        )
        for name, elapsed, limit in rows:
            print(f"{name}: {elapsed:.2f}s (limit {limit:.2f}s)")
        failed = [(name, elapsed, limit) for name, elapsed, limit in rows if elapsed > limit]
        if failed:
            details = ", ".join(
                f"{name} {elapsed:.2f}s > {limit:.2f}s"
                for name, elapsed, limit in failed
            )
            raise SystemExit(f"error: preflight limit exceeded: {details}")

    def prepare(self) -> None:
        self.run([self.samtools, "quickcheck", "-v", self.bam], label="quickcheck")
        header = command_output([self.samtools, "view", "-H", self.bam])
        (self.out / "benchmark.header.sam").write_text(header)
        if not any(
            line.startswith("@HD") and "SO:coordinate" in line.split("\t")
            for line in header.splitlines()
        ):
            raise SystemExit("error: BAM header does not declare SO:coordinate")

        environment = [
            f"date={datetime.now(timezone.utc).isoformat()}",
            command_output(["uname", "-a"]),
            Path("/etc/os-release").read_text(),
            command_output(["lscpu"]),
            command_output(["free", "-h"]),
            command_output(
                [
                    "lsblk",
                    "-o",
                    "NAME,MODEL,ROTA,SIZE,TYPE,FSTYPE,MOUNTPOINTS",
                ]
            ),
            command_output(["findmnt", "-T", str(self.bam)]),
            command_output(["findmnt", "-T", str(self.out)]),
        ]
        (self.out / "environment.txt").write_text("\n".join(environment))
        versions = [
            command_output([self.qbix, "--version"]),
            command_output([self.samtools, "--version"]),
        ]
        if "atlantool" in self.tools and self.have_atlantool:
            versions.append(command_output_optional([self.atlantool, "--version"]))
        if "bri" in self.tools and self.have_bri:
            versions.append(command_output_optional([self.bri, "--version"]))
        (self.out / "versions.txt").write_text("\n".join(versions))

        count = int(
            command_output([self.samtools, "view", "-@", "1", "-c", self.bam])
        )
        wanted = self.replicates * max(self.requested_query_sizes)
        selected = self.select_qnames(wanted)
        if len(selected) >= wanted:
            max_queries = max(self.requested_query_sizes)
        elif len(selected) >= self.replicates * 1_000:
            max_queries = 1_000
        else:
            raise SystemExit(
                f"error: need {self.replicates * 1_000} distinct QNAMEs; "
                f"found {len(selected)}"
            )
        selected = selected[: self.replicates * max_queries]
        self.write_queries(selected, max_queries)
        if self.profile == "full":
            self.write_absent_queries(max_queries)
        (self.out / "max_queries.txt").write_text(f"{max_queries}\n")
        upsert_tsv(
            self.run_dir / "datasets.tsv",
            [
                "dataset_id",
                "source_or_accession",
                "bam_path",
                "bam_bytes",
                "alignment_records",
                "sort_order",
                "sequencing_platform",
                "region",
                "storage_type",
                "notes",
            ],
            [
                (
                    self.dataset_id,
                    self.source,
                    self.bam,
                    self.bam.stat().st_size,
                    count,
                    "coordinate",
                    self.platform,
                    self.region,
                    self.storage,
                    f"{self.profile} benchmark",
                )
            ],
            key_column="dataset_id",
        )
        print(
            f"Preparation complete: {count} records, "
            f"{len(selected)} sampled QNAMEs, maximum query set {max_queries}"
        )

    def select_qnames(self, wanted: int) -> list[str]:
        command = [self.samtools, "view", "-@", "1", self.bam]
        self.log_command(command, "select-qnames")
        process = subprocess.Popen(
            [str(value) for value in command],
            stdout=subprocess.PIPE,
        )
        assert process.stdout is not None
        heap: list[tuple[int, str]] = []
        selected: set[str] = set()
        key = SEED.to_bytes(16, "little")
        for raw_line in process.stdout:
            raw_name = raw_line.split(b"\t", 1)[0]
            try:
                name = raw_name.decode()
            except UnicodeDecodeError as error:
                process.kill()
                raise SystemExit(f"error: non-UTF-8 QNAME: {error}") from error
            if name in selected:
                continue
            rank = int.from_bytes(
                hashlib.blake2b(raw_name, digest_size=16, key=key).digest(),
                "big",
            )
            item = (-rank, name)
            if len(heap) < wanted:
                heapq.heappush(heap, item)
                selected.add(name)
            elif item > heap[0]:
                _, removed = heapq.heapreplace(heap, item)
                selected.remove(removed)
                selected.add(name)
        if process.wait() != 0:
            raise SystemExit("error: samtools failed while selecting QNAMEs")
        return [name for _, name in sorted(heap, reverse=True)]

    def write_queries(self, names: list[str], max_queries: int) -> None:
        for old in self.queries.glob("rep*_n*.txt"):
            old.unlink()
        sizes = sorted({
            min(size, max_queries) for size in self.requested_query_sizes
        })
        for replicate in range(1, self.replicates + 1):
            start = (replicate - 1) * max_queries
            group = names[start : start + max_queries]
            for size in sizes:
                path = self.query_path(replicate, size, require=False)
                path.write_text("\n".join(group[:size]) + "\n")
        checksum_rows = [
            (path.name, sha256_file(path))
            for path in sorted(self.queries.glob("rep*_n*.txt"))
        ]
        write_tsv(
            self.queries / "checksums.tsv",
            ["file", "sha256"],
            checksum_rows,
        )

    def write_absent_queries(self, max_queries: int) -> None:
        all_names = []
        sizes = sorted({
            min(size, max_queries) for size in self.requested_query_sizes
        })
        for replicate in range(1, self.replicates + 1):
            names = [
                f"__QBIX_ABSENT_{SEED}_{replicate}_{i:08d}__"
                for i in range(max_queries)
            ]
            all_names.extend(names)
            for size in sizes:
                self.absent_query_path(replicate, size, require=False).write_text(
                    "\n".join(names[:size]) + "\n"
                )
        combined = self.tmp / "all-absent-qnames.txt"
        combined.write_text("\n".join(all_names) + "\n")
        completed = self.run(
            [self.samtools, "view", "-@", "1", "-c", "-N", combined, self.bam],
            label="verify-absent-qnames",
            stdout=subprocess.PIPE,
        )
        found = completed.stdout.decode().strip()
        if found != "0":
            raise SystemExit(
                f"error: generated absent QNAMEs matched {found} BAM records"
            )
        checksum_rows = [
            (path.name, sha256_file(path))
            for path in sorted(self.queries.glob("absent_rep*_n*.txt"))
        ]
        write_tsv(
            self.queries / "absent-checksums.tsv",
            ["file", "sha256"],
            checksum_rows,
        )

    def benchmark_index(self) -> None:
        self.warm_bam()
        output = self.out / "index_runs.tsv"
        rows = []
        for layout in self.qbix_layouts:
            qbi = self.qbi_paths[layout.name]
            for replicate in range(1, self.index_replicates + 1):
                qbi.unlink(missing_ok=True)
                temp_dir = self.tmp / f"qbix-{layout.name}"
                shutil.rmtree(temp_dir, ignore_errors=True)
                temp_dir.mkdir()
                timing = self.measure(
                    self.qbix_index_command(qbi, temp_dir, layout),
                    label=f"index-qbix-{layout.name}-{replicate}",
                    temp_paths=[temp_dir],
                )
                rows.append(
                    (
                        self.run_id,
                        self.dataset_id,
                        "qbix",
                        layout.index_format,
                        layout.radix_bits or "",
                        replicate,
                        "warm-not-cleared",
                        1,
                        1,
                        "512M",
                        8,
                        *index_timing_values(timing),
                        qbi.stat().st_size,
                    )
                )
                write_tsv(output, HEADER_INDEX_RUNS, rows)
        if "bri" in self.tools and self.have_bri:
            for replicate in range(1, self.index_replicates + 1):
                self.bri_index.unlink(missing_ok=True)
                timing = self.measure(
                    [self.bri, "index", self.bam],
                    label=f"index-bri-{replicate}",
                )
                rows.append(
                    (
                        self.run_id, self.dataset_id, "bri", "", "", replicate,
                        "warm-not-cleared", 1, 1, "", "",
                        *index_timing_values(timing), dir_size(self.bri_index),
                    )
                )
                write_tsv(output, HEADER_INDEX_RUNS, rows)
        if "atlantool" in self.tools and self.have_atlantool:
            for replicate in range(1, self.index_replicates + 1):
                shutil.rmtree(self.atlantool_index, ignore_errors=True)
                timing = self.measure(
                    [
                        self.atlantool, "index", self.bam,
                        "--index-path", self.atlantool_index,
                        "--thread-count", "1", "--force",
                    ],
                    label=f"index-atlantool-{replicate}",
                    temp_paths=[self.atlantool_index],
                )
                atlantool_bytes = dir_size(self.atlantool_index)
                atlantool_temp_bytes = max(0, timing.temp_bytes - atlantool_bytes)
                rows.append(
                    (
                        self.run_id, self.dataset_id, "atlantool", "", "", replicate,
                        "warm-not-cleared", 1, 1, "", "",
                        *index_timing_values(
                            timing, temp_bytes=atlantool_temp_bytes
                        ),
                        atlantool_bytes,
                    )
                )
                write_tsv(output, HEADER_INDEX_RUNS, rows)
        print(f"Index benchmark complete: {output}")

    def qbix_index_command(
        self,
        index: Path,
        temp_dir: Path,
        layout: QbixLayout | None = None,
        *,
        bgzf_threads: int = 1,
        sort_threads: int = 1,
        memory: str = "512M",
        bucket_bits: int = 8,
    ) -> list[str | Path]:
        layout = layout or self.qbix_layouts[0]
        command: list[str | Path] = [
            self.qbix,
            "index",
            "--index-format",
            layout.index_format,
            "--bgzf-threads",
            str(bgzf_threads),
            "--sort-threads",
            str(sort_threads),
            "--memory",
            memory,
            "--bucket-bits",
            str(bucket_bits),
            "--temp-dir",
            temp_dir,
            "-i",
            index,
            self.bam,
        ]
        if layout.radix_bits is not None:
            command[4:4] = ["--qbi2-radix-bits", str(layout.radix_bits)]
        return command

    def benchmark_parameters(self) -> None:
        self.warm_bam()
        conditions = [
            ("baseline", 1, 1, "512M", 8),
            ("bgzf-2", 2, 1, "512M", 8),
            ("bgzf-4", 4, 1, "512M", 8),
            ("bgzf-8", 8, 1, "512M", 8),
            ("sort-2", 1, 2, "512M", 8),
            ("sort-4", 1, 4, "512M", 8),
            ("memory-128M", 1, 1, "128M", 8),
            ("memory-2G", 1, 1, "2G", 8),
            ("bucket-10", 1, 1, "512M", 10),
            ("bucket-12", 1, 1, "512M", 12),
        ]
        output = self.out / "parameter_runs.tsv"
        header = ["condition", *HEADER_INDEX_RUNS]
        rows = []
        for condition, bgzf_threads, sort_threads, memory, bucket_bits in conditions:
            index = self.tmp / f"parameter-{condition}.qbi"
            temp_dir = self.tmp / f"parameter-{condition}-tmp"
            for replicate in range(1, self.index_replicates + 1):
                index.unlink(missing_ok=True)
                shutil.rmtree(temp_dir, ignore_errors=True)
                temp_dir.mkdir()
                timing = self.measure(
                    self.qbix_index_command(
                        index,
                        temp_dir,
                        self.parameter_layout,
                        bgzf_threads=bgzf_threads,
                        sort_threads=sort_threads,
                        memory=memory,
                        bucket_bits=bucket_bits,
                    ),
                    label=f"parameter-{condition}-{replicate}",
                    temp_paths=[temp_dir],
                )
                rows.append(
                    (
                        condition,
                        self.run_id,
                        self.dataset_id,
                        "qbix",
                        self.parameter_layout.index_format,
                        self.parameter_layout.radix_bits or "",
                        replicate,
                        "warm-not-cleared",
                        bgzf_threads,
                        sort_threads,
                        memory,
                        bucket_bits,
                        *index_timing_values(timing),
                        index.stat().st_size,
                    )
                )
                write_tsv(output, header, rows)
            index.unlink(missing_ok=True)
            shutil.rmtree(temp_dir, ignore_errors=True)
        print(f"Parameter benchmark complete: {output}")

    def check_correctness(self) -> None:
        self.require_index()
        names = self.query_path(1, self.max_queries())
        methods = self.comparison_methods()
        if "bri" in self.tools and self.have_bri:
            self.require_bri_index()
        if "atlantool" in self.tools and self.have_atlantool:
            self.require_atlantool_index()
        if "samtools" not in methods:
            methods = [*methods, "samtools"]
        commands = {method: self.query_command(method, names)[2] for method in methods}
        rows = []
        expected_hash: str | None = None
        expected_tool = "samtools"
        for tool, command in commands.items():
            path = self.tmp / f"check.{tool}.sam"
            canon_path = self.tmp / f"check.{tool}.canon.sam"
            with path.open("wb") as handle:
                self.run(command, label=f"correctness-{tool}", stdout=handle)
            records = count_lines(path)
            canonicalize_sam_file(path, canon_path)
            digest = sorted_sha256(canon_path)
            rows.append((self.run_id, self.dataset_id, tool, "present", records, digest))
            if tool == expected_tool:
                expected_hash = digest
        if self.profile == "full":
            absent = self.absent_query_path(1, self.max_queries())
            absent_methods = [
                f"qbix:{layout.name}:{order}"
                for layout in self.qbix_layouts
                for order in ("query", "bam")
            ]
            if "samtools" in self.tools:
                absent_methods.append("samtools")
            if "atlantool" in self.tools and self.have_atlantool:
                absent_methods.append("atlantool")
            if "bri" in self.tools and self.have_bri:
                absent_methods.append("bri")
            for method in absent_methods:
                tool, mode, command = self.query_command(method, absent)
                path = self.tmp / f"check.absent.{tool}.{mode}.sam"
                with path.open("wb") as handle:
                    self.run(
                        command,
                        label=f"correctness-absent-{tool}-{mode}",
                        stdout=handle,
                    )
                records = count_lines(path)
                rows.append(
                    (self.run_id, self.dataset_id, f"{tool}:{mode}", "absent", records, "")
                )
                if records != 0:
                    raise SystemExit(
                        f"error: absent-query correctness check failed for {tool} {mode}"
                    )
        output = self.out / "correctness.tsv"
        write_tsv(
            output,
            ["run_id", "dataset", "tool", "query_type", "records", "sorted_sha256"],
            rows,
        )
        if expected_hash is None:
            raise SystemExit("error: samtools reference check did not run")
        if any(row[-1] != expected_hash for row in rows if row[3] == "present"):
            raise SystemExit(f"error: correctness check failed; see {output}")
        state = {
            "bam_size": self.bam.stat().st_size,
            "bam_mtime_ns": self.bam.stat().st_mtime_ns,
            "index_sha256": {
                name: sha256_file(path) for name, path in self.qbi_paths.items()
            },
            "query_sha256": sha256_file(names),
            "absent_query_sha256": (
                sha256_file(self.absent_query_path(1, self.max_queries()))
                if self.profile == "full" else None
            ),
            "bri_index_sha256": (
                sha256_file(self.bri_index)
                if "bri" in self.tools and self.have_bri else None
            ),
            "atlantool_index_sha256": (
                sha256_file(next(self.atlantool_index.glob("qname.*.data.bgz")))
                if "atlantool" in self.tools and self.have_atlantool else None
            ),
        }
        write_json(self.out / "correctness-state.json", state)
        print(f"Correctness check passed: {output}")

    def benchmark_queries(self) -> None:
        self.require_index()
        if "bri" in self.tools and self.have_bri:
            self.require_bri_index()
        if "atlantool" in self.tools and self.have_atlantool:
            self.require_atlantool_index()
        self.require_current_correctness()
        self.warm_bam()
        max_queries = self.max_queries()
        output = self.out / "query_runs.tsv"
        methods = self.comparison_methods()
        orders = {}
        for replicate in range(1, self.replicates + 1):
            shift = (replicate - 1) * len(methods) // self.replicates
            orders[replicate] = methods[shift:] + methods[:shift]
        rows = []
        sizes = sorted({
            min(size, max_queries) for size in self.requested_query_sizes
        })
        for replicate in range(1, self.replicates + 1):
            for count in sizes:
                names = self.query_path(replicate, count)
                checksum = sha256_file(names)
                for method in orders[replicate]:
                    tool, mode, command = self.query_command(method, names)
                    index_format, radix_bits = self.query_layout_fields(method)
                    timing = self.measure(
                        command,
                        label=f"query-{tool}-{mode}-{count}-{replicate}",
                        stdout=subprocess.DEVNULL,
                    )
                    rows.append(
                        (
                            self.run_id,
                            self.dataset_id,
                            tool,
                            index_format,
                            radix_bits,
                            mode,
                            "present",
                            count,
                            replicate,
                            "warm-not-cleared",
                            *timing_values(timing),
                            "",
                            checksum,
                        )
                    )
                    write_tsv(
                        output,
                        HEADER_QUERY_RUNS,
                        rows,
                    )
        if self.profile == "full":
            absent_methods = [
                f"qbix:{layout.name}:{order}"
                for layout in self.qbix_layouts
                for order in ("query", "bam")
            ]
            if "samtools" in self.tools:
                absent_methods.append("samtools")
            if "atlantool" in self.tools and self.have_atlantool:
                absent_methods.append("atlantool")
            if "bri" in self.tools and self.have_bri:
                absent_methods.append("bri")
            absent_sizes = sorted({
                min(size, max_queries) for size in self.requested_query_sizes
            })
            for replicate in range(1, self.replicates + 1):
                for count in absent_sizes:
                    names = self.absent_query_path(replicate, count)
                    checksum = sha256_file(names)
                    for method in absent_methods:
                        tool, mode, command = self.query_command(method, names)
                        index_format, radix_bits = self.query_layout_fields(method)
                        timing = self.measure(
                            command,
                            label=f"query-absent-{tool}-{mode}-{count}-{replicate}",
                            stdout=subprocess.DEVNULL,
                        )
                        rows.append(
                            (
                                self.run_id,
                                self.dataset_id,
                                tool,
                                index_format,
                                radix_bits,
                                mode,
                                "absent",
                                count,
                                replicate,
                                "warm-not-cleared",
                                *timing_values(timing),
                                0,
                                checksum,
                            )
                        )
                        write_tsv(
                            output,
                            HEADER_QUERY_RUNS,
                            rows,
                        )
        print(f"Query benchmark complete: {output}")

    def query_command(
        self, method: str, names: Path
    ) -> tuple[str, str, list[str | Path]]:
        if method.startswith("qbix:"):
            _, layout_name, order = method.split(":")
            order_name = "query-order" if order == "query" else "bam-order"
            return (
                "qbix",
                f"{layout_name}-{order_name}",
                [
                    self.qbix,
                    "get",
                    f"--{order_name}",
                    "--bgzf-threads",
                    "1",
                    "-i",
                    self.qbi_paths[layout_name],
                    "-f",
                    names,
                    self.bam,
                ],
            )
        if method == "samtools":
            return (
                "samtools",
                "full-scan",
                [self.samtools, "view", "-@", "1", "-N", names, self.bam],
            )
        if method == "bri":
            # bri only supports a single QNAME per invocation, so batch queries
            # are answered by looping the CLI once per name.
            loop = (
                f"while IFS= read -r q; do "
                f"{shlex.quote(str(self.bri))} get {shlex.quote(str(self.bam))} \"$q\"; "
                f"done < {shlex.quote(str(names))}"
            )
            return ("bri", "loop-single-query", ["bash", "-c", loop])
        if method == "atlantool":
            return (
                "atlantool",
                "batch-file",
                [
                    self.atlantool, "view", self.bam,
                    "--index-path", self.atlantool_index,
                    "-f", names,
                ],
            )
        raise SystemExit(f"error: unknown query method: {method}")

    def query_layout_fields(self, method: str) -> tuple[str, int | str]:
        if not method.startswith("qbix:"):
            return "", ""
        _, layout_name, _ = method.split(":")
        layout = QBIX_LAYOUTS[layout_name]
        return layout.index_format, layout.radix_bits or ""

    def summary(self) -> None:
        index_rows = read_tsv(self.out / "index_runs.tsv")
        query_rows = read_tsv(self.out / "query_runs.tsv")
        index_groups: dict[tuple[str, str, str], list[dict[str, str]]] = {}
        for row in index_rows:
            key = (row["tool"], row["index_format"], row["radix_bits"])
            index_groups.setdefault(key, []).append(row)
        index_summary_rows = []
        for (tool, index_format, radix_bits), tool_rows in sorted(index_groups.items()):
            elapsed = [float(row["elapsed_s"]) for row in tool_rows]
            index_summary_rows.append(
                (
                    self.run_id,
                    self.dataset_id,
                    tool,
                    index_format,
                    radix_bits,
                    statistics.median(elapsed),
                    min(elapsed),
                    max(elapsed),
                    max(int(row["max_rss_kb"]) for row in tool_rows),
                    max(
                        int(row["temp_bytes"])
                        for row in tool_rows
                        if row.get("temp_bytes", "") != ""
                    )
                    if any(row.get("temp_bytes", "") != "" for row in tool_rows)
                    else "",
                    tool_rows[-1]["index_bytes"],
                )
            )
        write_tsv(
            self.out / "index_summary.tsv",
            [
                "run_id",
                "dataset",
                "tool",
                "index_format",
                "radix_bits",
                "median_s",
                "min_s",
                "max_s",
                "peak_rss_kb",
                "max_temp_bytes",
                "index_bytes",
            ],
            index_summary_rows,
        )
        groups: dict[tuple[str, str, str, str, str, str], list[float]] = {}
        for row in query_rows:
            key = (
                row["tool"], row["index_format"], row["radix_bits"], row["mode"],
                row["query_type"], row["query_count"],
            )
            groups.setdefault(key, []).append(float(row["elapsed_s"]))
        summary_rows = []
        for (
            tool, index_format, radix_bits, mode, query_type, count
        ), values in sorted(groups.items()):
            summary_rows.append(
                (
                    self.run_id,
                    self.dataset_id,
                    tool,
                    index_format,
                    radix_bits,
                    mode,
                    query_type,
                    count,
                    statistics.median(values),
                    min(values),
                    max(values),
                )
            )
        write_tsv(
            self.out / "query_summary.tsv",
            [
                "run_id",
                "dataset",
                "tool",
                "index_format",
                "radix_bits",
                "mode",
                "query_type",
                "query_count",
                "median_s",
                "min_s",
                "max_s",
            ],
            summary_rows,
        )
        self.plot_query_summary(summary_rows)
        self.write_paper_markdown()
        print(f"Summary complete: {self.out}")

    def write_paper_markdown(self) -> None:
        dataset_rows = {
            row["dataset_id"]: row
            for row in read_tsv(self.run_dir / "datasets.tsv")
        }
        completed = [
            path for path in sorted(self.run_dir.iterdir())
            if (path / "index_summary.tsv").is_file()
            and (path / "query_summary.tsv").is_file()
        ]
        lines = [
            "# Paper Benchmark Tables",
            "",
            f"Run ID: `{self.run_id}`",
            "",
            "Primary qbix layout for paper tables: `qbi2-p16`.",
            "",
            "Values are medians across completed replicates unless noted. "
            "Raw commands, manifests, environment records, query files, and "
            "per-replicate TSVs are stored in each dataset directory.",
            "",
            "## インデックス構築",
            "",
            "| データセット | ツール | 構築時間 | 最大RSS | 一時ディスク | インデックスサイズ |",
            "|:--|:--|--:|--:|--:|--:|",
        ]
        for dataset_dir in completed:
            dataset_id = dataset_dir.name
            index_rows = read_tsv(dataset_dir / "index_summary.tsv")
            label = paper_dataset_label(dataset_rows.get(dataset_id, {}), dataset_id)
            for offset, (tool, index_format, radix_bits, display_tool) in enumerate(
                [
                    ("qbix", "qbi2", "16", "qbix"),
                    ("atlantool", "", "", "Atlantool"),
                ]
            ):
                row = find_summary_row(
                    index_rows,
                    tool=tool,
                    index_format=index_format,
                    radix_bits=radix_bits,
                )
                lines.append(
                    "| "
                    + " | ".join(
                        [
                            label if offset == 0 else "",
                            display_tool,
                            format_seconds(row.get("median_s") if row else None),
                            format_kib(row.get("peak_rss_kb") if row else None),
                            format_bytes(row.get("max_temp_bytes") if row else None),
                            format_bytes(row.get("index_bytes") if row else None),
                        ]
                    )
                    + " |"
                )
        lines.extend(
            [
                "",
                "## 完全な検索処理",
                "",
                "| データセット | QNAME数 | ツールとモード | 存在するQNAME | 存在しないQNAME |",
                "|:--|--:|:--|--:|--:|",
            ]
        )
        query_methods = [
            ("qbix", "qbi2", "16", "qbi2-p16-query-order", "qbix, query order"),
            ("qbix", "qbi2", "16", "qbi2-p16-bam-order", "qbix, BAM order"),
            ("atlantool", "", "", "batch-file", "Atlantool"),
            ("samtools", "", "", "full-scan", "samtools全走査"),
        ]
        for dataset_dir in completed:
            dataset_id = dataset_dir.name
            query_rows = read_tsv(dataset_dir / "query_summary.tsv")
            label = paper_dataset_label(dataset_rows.get(dataset_id, {}), dataset_id)
            sizes = sorted(
                {int(row["query_count"]) for row in query_rows},
                key=int,
            )
            for size_index, size in enumerate(sizes):
                for method_index, (
                    tool, index_format, radix_bits, mode, display_tool
                ) in enumerate(query_methods):
                    present = find_summary_row(
                        query_rows,
                        tool=tool,
                        index_format=index_format,
                        radix_bits=radix_bits,
                        mode=mode,
                        query_type="present",
                        query_count=str(size),
                    )
                    absent = find_summary_row(
                        query_rows,
                        tool=tool,
                        index_format=index_format,
                        radix_bits=radix_bits,
                        mode=mode,
                        query_type="absent",
                        query_count=str(size),
                    )
                    lines.append(
                        "| "
                        + " | ".join(
                            [
                                label if size_index == 0 and method_index == 0 else "",
                                f"{size:,}" if method_index == 0 else "",
                                display_tool,
                                format_seconds(
                                    present.get("median_s") if present else None
                                ),
                                format_seconds(
                                    absent.get("median_s") if absent else None
                                ),
                            ]
                        )
                        + " |"
                    )
        lines.extend(
            [
                "",
                "## 正確性",
                "",
                "All present-query tool outputs are normalized and compared against "
                "`samtools view -N`; absent-query outputs must contain zero records. "
                "See each dataset's `correctness.tsv` and `correctness-state.json`.",
                "",
            ]
        )
        (self.run_dir / "paper_tables.md").write_text("\n".join(lines))

    def plot_query_summary(self, rows: Sequence[Sequence[object]]) -> None:
        matplotlib_config = self.tmp / "matplotlib"
        matplotlib_config.mkdir(exist_ok=True)
        os.environ.setdefault("MPLCONFIGDIR", str(matplotlib_config))
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt

        series_by_type: dict[
            str, dict[tuple[str, str], list[tuple[int, float, float, float]]]
        ] = {}
        for row in rows:
            tool = str(row[2])
            mode = str(row[5])
            query_type = str(row[6])
            series = series_by_type.setdefault(query_type, {})
            series.setdefault((tool, mode), []).append(
                (int(row[7]), float(row[8]), float(row[9]), float(row[10]))
            )
        for query_type, series in series_by_type.items():
            figure, axes = plt.subplots(figsize=(7.2, 4.6))
            for (tool, mode), values in sorted(series.items()):
                values.sort()
                x = [value[0] for value in values]
                median = [max(value[1], PLOT_TIME_FLOOR_SECONDS) for value in values]
                lower = [
                    center - max(value[2], PLOT_TIME_FLOOR_SECONDS)
                    for center, value in zip(median, values)
                ]
                upper = [
                    max(value[3], PLOT_TIME_FLOOR_SECONDS) - center
                    for center, value in zip(median, values)
                ]
                axes.errorbar(
                    x,
                    median,
                    yerr=[lower, upper],
                    marker="o",
                    capsize=3,
                    label=f"{tool} {mode}",
                )
            axes.set_xscale("log")
            axes.set_yscale("log")
            axes.set_xlabel("QNAMEs")
            axes.set_ylabel("Wall-clock time (s)")
            axes.grid(True, which="both", alpha=0.25)
            axes.legend(fontsize="small")
            figure.tight_layout()
            filename = (
                "query_time.pdf"
                if self.profile == "quick" and query_type == "present"
                else f"query_time_{query_type}.pdf"
            )
            figure.savefig(self.out / filename)
            plt.close(figure)

    def query_path(self, replicate: int, count: int, *, require: bool = True) -> Path:
        path = self.queries / f"rep{replicate}_n{count:05d}.txt"
        if require and not path.is_file():
            raise SystemExit(f"error: missing query file: {path}")
        return path

    def absent_query_path(
        self, replicate: int, count: int, *, require: bool = True
    ) -> Path:
        path = self.queries / f"absent_rep{replicate}_n{count:05d}.txt"
        if require and not path.is_file():
            raise SystemExit(f"error: missing absent query file: {path}")
        return path

    def max_queries(self) -> int:
        path = self.out / "max_queries.txt"
        if not path.is_file():
            raise SystemExit(f"error: missing {path}; run prepare first")
        return int(path.read_text().strip())

    def require_index(self) -> None:
        missing = [path for path in self.qbi_paths.values() if not path.is_file()]
        if missing:
            raise SystemExit(f"error: missing {missing[0]}; run index first")

    def require_bri_index(self) -> None:
        if not self.bri_index.is_file():
            raise SystemExit(f"error: missing {self.bri_index}; run index first")

    def require_atlantool_index(self) -> None:
        if not self.atlantool_index.is_dir():
            raise SystemExit(f"error: missing {self.atlantool_index}; run index first")

    def require_current_correctness(self) -> None:
        path = self.out / "correctness-state.json"
        if not path.is_file():
            raise SystemExit("error: run correctness check first")
        state = json.loads(path.read_text())
        expected = {
            "bam_size": self.bam.stat().st_size,
            "bam_mtime_ns": self.bam.stat().st_mtime_ns,
            "index_sha256": {
                name: sha256_file(path) for name, path in self.qbi_paths.items()
            },
            "query_sha256": sha256_file(self.query_path(1, self.max_queries())),
            "absent_query_sha256": (
                sha256_file(self.absent_query_path(1, self.max_queries()))
                if self.profile == "full" else None
            ),
            "bri_index_sha256": (
                sha256_file(self.bri_index)
                if "bri" in self.tools and self.have_bri else None
            ),
            "atlantool_index_sha256": (
                sha256_file(next(self.atlantool_index.glob("qname.*.data.bgz")))
                if "atlantool" in self.tools and self.have_atlantool else None
            ),
        }
        if state != expected:
            raise SystemExit("error: correctness results are stale; run check again")

    def warm_bam(self) -> None:
        with self.bam.open("rb") as handle:
            for _ in iter(lambda: handle.read(16 * 1024 * 1024), b""):
                pass


def validate_id(value: str, what: str) -> str:
    if not ID_PATTERN.fullmatch(value):
        raise SystemExit(
            f"error: {what} must contain only letters, digits, '.', '_' or '-'"
        )
    return value


def require_command(command: str) -> None:
    if shutil.which(command) is None:
        raise SystemExit(f"error: required command not found: {command}")


def command_output(command: Sequence[str | Path]) -> str:
    return subprocess.run(
        [str(value) for value in command],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout


def command_output_optional(command: Sequence[str | Path]) -> str:
    completed = subprocess.run(
        [str(value) for value in command],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if completed.returncode == 0:
        return completed.stdout
    return (
        f"{shlex.join(str(value) for value in command)} failed with "
        f"exit status {completed.returncode}\n{completed.stdout}"
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sorted_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    process = subprocess.Popen(["sort", str(path)], stdout=subprocess.PIPE)
    assert process.stdout is not None
    for chunk in iter(lambda: process.stdout.read(1024 * 1024), b""):
        digest.update(chunk)
    if process.wait() != 0:
        raise SystemExit(f"error: sort failed for {path}")
    return digest.hexdigest()


def dir_size(path: Path) -> int:
    if path.is_dir():
        return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())
    return path.stat().st_size


def paths_size(paths: Sequence[Path]) -> int:
    total = 0
    for path in paths:
        try:
            if path.is_dir():
                total += sum(f.stat().st_size for f in path.rglob("*") if f.is_file())
            elif path.exists():
                total += path.stat().st_size
        except FileNotFoundError:
            continue
    return total


def find_summary_row(
    rows: Sequence[dict[str, str]], **expected: str
) -> dict[str, str] | None:
    for row in rows:
        if all(row.get(key, "") == value for key, value in expected.items()):
            return row
    return None


def paper_dataset_label(dataset: dict[str, str], fallback: str) -> str:
    platform = dataset.get("sequencing_platform", "")
    haystack = f"{fallback} {platform}".lower()
    if "illumina" in haystack:
        return "Illumina"
    if "pacbio" in haystack or "hifi" in haystack:
        return "PacBio HiFi"
    if "ont" in haystack or "nanopore" in haystack:
        return "Oxford Nanopore"
    return fallback


def format_seconds(value: str | None) -> str:
    if value in (None, ""):
        return "`[未測定]`"
    seconds = float(value)
    if seconds < 0.01:
        return f"{seconds:.4f} s"
    if seconds < 10:
        return f"{seconds:.3f} s"
    return f"{seconds:.2f} s"


def format_bytes(value: str | None) -> str:
    if value in (None, ""):
        return "`[未測定]`"
    size = float(value)
    units = ["B", "KiB", "MiB", "GiB", "TiB"]
    unit = units[0]
    for unit in units:
        if size < 1024 or unit == units[-1]:
            break
        size /= 1024
    if unit == "B":
        return f"{int(size)} {unit}"
    return f"{size:.1f} {unit}"


def format_kib(value: str | None) -> str:
    if value in (None, ""):
        return "`[未測定]`"
    return format_bytes(str(int(value) * 1024))


def canonicalize_sam_line(line: bytes) -> bytes:
    # Field order for mandatory columns 1-11 is fixed by the SAM spec; optional
    # tags (12+) are not ordered, so tools may emit them in different order.
    # Float-valued tags (ec, rq, sn, ...) are dropped rather than compared:
    # htslib prints them with fewer significant digits than some other tools,
    # so byte-identical formatting can't be expected even for equal values.
    fields = line.rstrip(b"\n").split(b"\t")
    if len(fields) > 11:
        kept = []
        for field in fields[11:]:
            parts = field.split(b":", 2)
            is_float = len(parts) == 3 and (
                parts[1] == b"f" or (parts[1] == b"B" and parts[2].startswith(b"f,"))
            )
            if not is_float:
                kept.append(field)
        fields = fields[:11] + sorted(kept, key=lambda f: f.split(b":", 1)[0])
    return b"\t".join(fields) + b"\n"


def canonicalize_sam_file(src: Path, dst: Path) -> None:
    with src.open("rb") as fin, dst.open("wb") as fout:
        for line in fin:
            fout.write(canonicalize_sam_line(line))


def count_lines(path: Path) -> int:
    with path.open("rb") as handle:
        return sum(1 for _ in handle)


def timing_values(timing: Timing) -> tuple[float, float, float, int, int]:
    return (
        timing.elapsed_s,
        timing.user_s,
        timing.sys_s,
        timing.max_rss_kb,
        timing.exit_status,
    )


def index_timing_values(
    timing: Timing, *, temp_bytes: int | None = None
) -> tuple[float, float, float, int, int, int]:
    return (
        *timing_values(timing),
        timing.temp_bytes if temp_bytes is None else temp_bytes,
    )


def write_tsv(path: Path, header: Sequence[str], rows: Sequence[Sequence[object]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def upsert_tsv(
    path: Path,
    header: Sequence[str],
    rows: Sequence[Sequence[object]],
    *,
    key_column: str,
) -> None:
    existing = read_tsv(path) if path.exists() else []
    key_index = list(header).index(key_column)
    replacements = {str(row[key_index]): row for row in rows}
    retained = [
        tuple(row.get(column, "") for column in header)
        for row in existing
        if row[key_column] not in replacements
    ]
    write_tsv(path, header, [*retained, *replacements.values()])


def read_tsv(path: Path) -> list[dict[str, str]]:
    if not path.is_file():
        raise SystemExit(f"error: missing {path}")
    with path.open(newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def default_dataset_id(bam: Path) -> str:
    value = re.sub(r"[^A-Za-z0-9._-]+", "-", bam.stem)
    return value or "dataset"


def default_tools() -> list[str]:
    value = os.environ.get("QBIX_BENCHMARK_TOOLS")
    if value:
        return [tool for tool in re.split(r"[,\s]+", value.strip()) if tool]
    return ["qbix", "samtools", "atlantool"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "stage",
        choices=(
            "preflight", "prepare", "index", "check", "queries", "parameters",
            "summary",
        ),
    )
    parser.add_argument("bam", type=Path)
    parser.add_argument("--run-id", default=os.environ.get("QBIX_RUN_ID"))
    parser.add_argument("--dataset-id", default=os.environ.get("QBIX_DATASET_ID"))
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT_ROOT)
    parser.add_argument("--max-index-s", type=float, default=60.0)
    parser.add_argument("--max-scan-s", type=float, default=15.0)
    parser.add_argument("--profile", choices=("quick", "full"), default="quick")
    parser.add_argument(
        "--qbix-layouts",
        nargs="+",
        choices=tuple(QBIX_LAYOUTS),
        help="qbix layouts to benchmark (profile-dependent default)",
    )
    parser.add_argument(
        "--parameter-layout",
        choices=tuple(QBIX_LAYOUTS),
        default="qbi2-p16",
        help="qbix layout used by the optional parameter stage",
    )
    parser.add_argument(
        "--tools",
        nargs="+",
        choices=("qbix", "samtools", "bri", "atlantool"),
        default=default_tools(),
        help="tools to benchmark (or set QBIX_BENCHMARK_TOOLS)",
    )
    parser.add_argument("--platform", default="unspecified")
    parser.add_argument("--source", default="unspecified")
    parser.add_argument("--region", default="unspecified")
    parser.add_argument("--storage", default="local-storage-unspecified")
    args = parser.parse_args()
    if not args.run_id:
        parser.error("--run-id is required (or set QBIX_RUN_ID)")
    args.dataset_id = args.dataset_id or default_dataset_id(args.bam)
    return args


def main() -> None:
    args = parse_args()
    benchmark = Benchmark(args)
    actions = {
        "preflight": lambda: benchmark.preflight(
            args.max_index_s, args.max_scan_s
        ),
        "prepare": benchmark.prepare,
        "index": benchmark.benchmark_index,
        "check": benchmark.check_correctness,
        "queries": benchmark.benchmark_queries,
        "parameters": benchmark.benchmark_parameters,
        "summary": benchmark.summary,
    }
    actions[args.stage]()


if __name__ == "__main__":
    main()
