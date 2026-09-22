# Paper benchmarks

This directory contains three reproducible benchmark profiles for the qbix
paper.

- `quick` preserves the original chromosome-subset experiment: three query
  replicates, 1/100/10,000 present QNAMEs, using QBI1.
- `paper` is the whole-genome manuscript profile. It measures QBI2 with
  `P = 16`, uses three index replicates and five disjoint query sets, and
  tests 1/100/10,000 present QNAMEs plus 10,000 absent QNAMEs. This is the
  recommended profile for replacing the current chromosome 21 results.
- `full` retains the exploratory matrix of QBI1 and QBI2 (`P = 8/12/16`) for
  1/10/100/1,000/10,000 present and absent QNAMEs. It is intentionally much
  slower and is not needed for the manuscript tables.

The BAM path does not imply a chromosome. Each run records the region and
provenance in its manifest and run-wide dataset table.

## Requirements

- Linux
- GNU time 1.9 (installed by pixi)
- Python 3
- SAMtools
- Atlantool (installed by `./setup_tools.sh`)
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
./setup_tools.sh
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

Use local SSD or NVMe storage. Allow at least 750 GiB for the three source BAMs,
indexes, temporary construction files, and results. The benchmark records the
mount and storage description used for both input and output.

## Run

Builds and generated results are written below `output/`.

```sh
cd paper/work
pixi run benchmark /path/to/benchmark.bam
```

For one manuscript dataset, supply provenance through environment variables:

```sh
QBIX_DATASET_PLATFORM='ONT R10.4.1 SUP' \
QBIX_DATASET_SOURCE='public accession or URL' \
QBIX_DATASET_REGION='whole-genome' \
QBIX_STORAGE='local NVMe, ext4' \
  pixi run benchmark-paper /path/to/benchmark.bam ont-wgs run-01
```

After downloading the three configured HG002 inputs, the complete manuscript
benchmark is one command:

```sh
QBIX_STORAGE='local NVMe, ext4' pixi run benchmark-paper-all run-01
```

The datasets run serially under one run ID, starting with the smallest. The
`paper` profile is sized to make a same-day run practical on the target
8-core/64-GiB machine; actual duration depends mainly on storage throughput.

The runner creates a UTC timestamp run ID. An explicit dataset ID and run ID
can be supplied when needed:

```sh
pixi run benchmark /path/to/benchmark.bam dataset-L trial-01
```

Preflight stops the run when qbix indexing exceeds 60 seconds or a SAMtools
scan exceeds 15 seconds. Override these limits explicitly for a deliberate
larger run. The paper runner uses 7,200 and 1,800 seconds, respectively.

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

Add `--profile paper` to every separately invoked stage for a
manuscript run. Use `--profile full` only for the exploratory matrix. The
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

Present QNAMEs are selected deterministically across the complete BAM. SAMtools
first performs QNAME-based subsampling with seed 20260730, targeting about 20
times the required number of records. The workflow then retains the lowest
keyed BLAKE2b ranks among distinct QNAMEs and divides them into disjoint query
sets. If the first sample contains too few distinct names, the sampling
fraction is doubled and retried. This avoids formatting the entire WGS BAM as
SAM in Python while preserving whole-file sampling.

The default comparison tools are `qbix`, `samtools`, and `atlantool`, matching
the paper tables. `bri` remains available as an explicit extra comparison:

```sh
QBIX_BENCHMARK_TOOLS='qbix samtools atlantool bri' \
  pixi run benchmark-paper /path/to/benchmark.bam dataset-L trial-01
```

## Use a completed run in the manuscript

Every summary refreshes `output/RUN_ID/paper_tables.md` from all completed
datasets in that run. After reviewing the raw TSV files and this table, update
the English and Japanese evaluation sections and regenerate the figure from the
reviewed values. This interpretive step is intentionally not automatic.

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
- `run_paper_benchmark.sh`: runs the same-day manuscript profile for one BAM
- `run_paper_benchmarks.sh`: runs all three configured whole-genome BAMs
- `run_full_benchmark.sh`: runs the exploratory QBI layout/query matrix
- `download_real_data.sh`: downloads and verifies the real HG002 datasets
- `benchmark.py`: prepares data, runs measurements, verifies output, and summarizes results
- `pixi.toml` / `pixi.lock`: pinned Python and SAMtools environment

Comparison tools can be added to individual stages without changing query
generation or the qbix measurements. Correctness is checked against
`samtools view -N` after normalizing record order and SAM optional-tag order.
