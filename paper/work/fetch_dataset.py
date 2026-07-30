#!/usr/bin/env python3
"""Reproducibly fetch a chromosome subset from a remote coordinate-sorted BAM.

Extracts a region via htslib range requests (no full-file download), then
records a manifest with everything needed to reproduce or verify the subset:
source URL, extraction command, samtools/htslib version, record counts, and
checksums of the resulting files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

WORK_DIR = Path(__file__).resolve().parent
DEFAULT_DATA_DIR = WORK_DIR / "data"
NAME_PATTERN = re.compile(r"^[A-Za-z0-9._-]+$")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, check=True, **kwargs)


def output_of(command: list[str]) -> str:
    return subprocess.run(command, check=True, stdout=subprocess.PIPE, text=True).stdout


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True, help="dataset name, e.g. HG002-PacBio-HiFi-chr21")
    parser.add_argument("--url", required=True, help="source BAM URL (remote, BAI must sit alongside it)")
    parser.add_argument("--region", required=True, help="samtools region to extract, e.g. chr21")
    parser.add_argument("--platform", required=True, help="sequencing platform, e.g. 'PacBio HiFi Revio'")
    parser.add_argument("--source-md5", help="published MD5 of the full source BAM, if known")
    parser.add_argument("--source-bai-md5", help="published MD5 of the full source BAI, if known")
    parser.add_argument("--data-dir", type=Path, default=DEFAULT_DATA_DIR)
    parser.add_argument("--samtools", default=os.environ.get("SAMTOOLS", "samtools"))
    args = parser.parse_args()

    if not NAME_PATTERN.fullmatch(args.name):
        raise SystemExit("error: --name must contain only letters, digits, '.', '_' or '-'")

    args.data_dir = args.data_dir.resolve()
    args.data_dir.mkdir(parents=True, exist_ok=True)
    bam_path = args.data_dir / f"{args.name}.bam"
    bai_path = args.data_dir / f"{args.name}.bam.bai"
    header_path = args.data_dir / f"{args.name}.header.sam"
    manifest_path = args.data_dir / f"{args.name}.manifest.json"

    extract_command = [
        args.samtools, "view", "-bh",
        "-o", str(bam_path),
        args.url, args.region,
    ]
    fetched_at = datetime.now(timezone.utc).isoformat()
    print(f"Extracting {args.region} from {args.url}")
    # htslib saves a remote BAM's index in its current directory. Keep that
    # source-only cache away from the benchmark artifacts and remove it after
    # the regional extraction completes.
    with tempfile.TemporaryDirectory(prefix=".source-index-", dir=args.data_dir) as index_cache:
        run(extract_command, cwd=index_cache)

    run([args.samtools, "index", str(bam_path)])
    run([args.samtools, "quickcheck", "-v", str(bam_path)])

    header = output_of([args.samtools, "view", "-H", str(bam_path)])
    header_path.write_text(header)
    so_coordinate = any(
        line.startswith("@HD") and "SO:coordinate" in line.split("\t")
        for line in header.splitlines()
    )

    record_count = int(output_of([args.samtools, "view", "-@", "1", "-c", str(bam_path)]).strip())
    flagstat = output_of([args.samtools, "flagstat", str(bam_path)])

    view = subprocess.Popen(
        [args.samtools, "view", "-@", "1", str(bam_path)], stdout=subprocess.PIPE
    )
    count_qnames = subprocess.Popen(
        ["awk", "-F\t", "!seen[$1]++ { n++ } END { print n+0 }"],
        stdin=view.stdout, stdout=subprocess.PIPE, text=True,
    )
    view.stdout.close()
    distinct_qnames_out, _ = count_qnames.communicate()
    if view.wait() != 0 or count_qnames.wait() != 0:
        raise SystemExit("error: failed to count distinct QNAMEs")
    distinct_qnames = int(distinct_qnames_out.strip())

    samtools_version = output_of([args.samtools, "--version"]).splitlines()[0]

    manifest = {
        "dataset_name": args.name,
        "platform": args.platform,
        "fetched_at": fetched_at,
        "source_url": args.url,
        "source_bam_md5": args.source_md5,
        "source_bai_md5": args.source_bai_md5,
        "region": args.region,
        "extraction_command": extract_command,
        "samtools_version": samtools_version,
        "bam_path": str(bam_path.relative_to(WORK_DIR)),
        "bam_bytes": bam_path.stat().st_size,
        "bam_sha256": sha256_file(bam_path),
        "bai_bytes": bai_path.stat().st_size,
        "bai_sha256": sha256_file(bai_path),
        "header_so_coordinate": so_coordinate,
        "alignment_records": record_count,
        "distinct_qnames": distinct_qnames,
        "flagstat": flagstat.splitlines(),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"Subset ready: {bam_path} ({manifest['bam_bytes']} bytes, {record_count} records)")
    print(f"Manifest: {manifest_path}")


if __name__ == "__main__":
    main()
