# Paper benchmarks

This directory contains two reproducible benchmark profiles for the qbix paper.

- `quick` preserves the original chromosome-subset experiment: three query
  replicates, 1/100/10,000 present QNAMEs, using QBI1.
- `full` is the paper-table profile: three index replicates and five query
  replicates for the Markdown benchmark tables. It measures qbix and Atlantool
  index construction, plus qbix query-order, qbix BAM-order, Atlantool, and
  `samtools view -N` full-scan lookups for 1/10/100/1,000/10,000 present and
  absent QNAMEs.

The BAM path does not imply a chromosome. The earlier published measurements
used HG002 chr21 subsets; the full profile accepts chromosome subsets or
whole-genome BAMs and records the choice in its manifest.

## Requirements

- Linux
- GNU time 1.9 (installed by pixi)
- Python 3
- SAMtools
- a coordinate-sorted BAM on local storage

`hyperfine` is not required. `/usr/bin/time` is used because the benchmark
records elapsed time, CPU time, and peak RSS together. Index stages also sample
tool working directories to estimate peak temporary disk use.

## Download real data

The paper datasets can be downloaded with resumable transfers. `aria2c` is
preferred; the script falls back to `wget -c`. With no arguments it downloads
all three whole-genome BAMs (about 480 GiB in total, including the already
downloaded PacBio file):

```sh
cd paper/work
./download_real_data.sh
```

Datasets can also be selected independently. Completed files are checked by
published MD5 where available and always checked against the remote byte size.

```sh
./download_real_data.sh ont illumina
```

The sources are HG002 PacBio HiFi Revio 48x from GIAB, HG002 ONT R10.4.1 SUP
from Oxford Nanopore Open Data, and HG002 Illumina HiSeq 2x250 Novoalign from
GIAB. Files are stored under `data/`; large BAM, BAI, and aria2 control files
are excluded from Git.

## Run

Builds and generated results are written below `output/`.

```sh
cd paper/work
pixi run benchmark /path/to/benchmark.bam
```

For the full profile, supply dataset provenance through environment variables:

```sh
QBIX_DATASET_PLATFORM='ONT R10.4.1 SUP' \
QBIX_DATASET_SOURCE='public accession or URL' \
QBIX_DATASET_REGION='whole-genome' \
QBIX_STORAGE='local NVMe, ext4' \
  pixi run benchmark-full /path/to/benchmark.bam ont-wgs run-01
```

Run the full profile on at least one Illumina paired-end, one PacBio HiFi, and
one ONT dataset. At least one dataset should be whole-genome; chromosome-only
results must be labeled as such.

The runner creates a UTC timestamp run ID. An explicit dataset ID and run ID
can be supplied when needed:

```sh
pixi run benchmark /path/to/benchmark.bam dataset-L trial-01
```

Preflight stops the run when qbix indexing exceeds 60 seconds or a SAMtools
scan exceeds 15 seconds. Override these limits explicitly for a deliberate
larger run:

```sh
QBIX_MAX_INDEX_S=180 QBIX_MAX_SCAN_S=60 \
  pixi run benchmark /path/to/benchmark.bam dataset-L trial-01
```

The stages can also be run separately:

```sh
pixi run preflight BAM --run-id trial-01 --dataset-id dataset-L
pixi run prepare   BAM --run-id trial-01 --dataset-id dataset-L
pixi run index     BAM --run-id trial-01 --dataset-id dataset-L
pixi run check     BAM --run-id trial-01 --dataset-id dataset-L
pixi run queries   BAM --run-id trial-01 --dataset-id dataset-L
pixi run summary   BAM --run-id trial-01 --dataset-id dataset-L
```

Add `--profile full` to every separately invoked stage for a full run. The
optional one-factor-at-a-time construction experiment is run separately:

```sh
pixi run python benchmark.py parameters BAM \
  --profile full --run-id trial-01 --dataset-id dataset-L
```

It measures QBI2 P=16 at the baseline (`bgzf=1`, `sort=1`, `memory=512M`,
`bucket_bits=8`) and varies BGZF threads, sort threads, memory, and bucket bits
one at a time. It is intentionally excluded from `benchmark-full` because it
adds 30 index builds per dataset.

Tool paths can be overridden with the `QBIX`, `SAMTOOLS`, `PYTHON`, and
`TIME_BIN` environment variables.

The BAM is read once before query timing. The operating-system page cache is
not explicitly cleared between runs.

The default comparison tools are `qbix`, `samtools`, and `atlantool`, matching
the paper tables. `bri` remains available as an explicit extra comparison:

```sh
QBIX_BENCHMARK_TOOLS='qbix samtools atlantool bri' \
  pixi run benchmark-full /path/to/benchmark.bam dataset-L trial-01
```

Results are isolated by run and dataset:

```text
output/
└── RUN_ID/
    ├── datasets.tsv
    ├── paper_tables.md             # Markdown tables for the paper draft
    └── DATASET_ID/
        ├── manifest.json
        ├── commands.jsonl
        ├── preflight.tsv
        ├── index_runs.tsv
        ├── query_runs.tsv
        ├── parameter_runs.tsv        # optional
        ├── correctness.tsv
        ├── index_summary.tsv
        ├── query_summary.tsv
        ├── query_time_present.pdf      # full profile
        ├── query_time_absent.pdf       # full profile
        └── query_time.pdf              # quick profile
```

## Layout

- `run_quick_benchmark.sh`: runs all stages
- `run_full_benchmark.sh`: runs the full QBI layout/query matrix
- `download_real_data.sh`: downloads and verifies the real HG002 datasets
- `benchmark.py`: prepares data, runs measurements, verifies output, and summarizes results
- `pixi.toml` / `pixi.lock`: pinned Python and SAMtools environment

Comparison tools can be added to individual stages without changing query
generation or the qbix measurements. Correctness is checked against
`samtools view -N` after normalizing record order and SAM optional-tag order.
