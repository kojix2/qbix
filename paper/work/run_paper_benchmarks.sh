#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
DATA_DIR=${QBIX_DATA_DIR:-"$SCRIPT_DIR/data"}

[[ $# -le 1 ]] || {
    printf 'usage: %s [RUN_ID]\n' "$0" >&2
    exit 1
}

RUN_ID=${1:-$(date -u +%Y%m%dT%H%M%SZ)}
RUNNER="$SCRIPT_DIR/run_paper_benchmark.sh"

run_dataset() {
    local dataset_id=$1
    local platform=$2
    local source=$3
    local bam=$4

    if [[ ! -f "$bam" ]]; then
        printf 'error: missing BAM: %s\n' "$bam" >&2
        printf 'Run %s/download_real_data.sh first.\n' "$SCRIPT_DIR" >&2
        exit 1
    fi

    QBIX_DATASET_PLATFORM=$platform \
    QBIX_DATASET_SOURCE=$source \
    QBIX_DATASET_REGION=whole-genome \
    "$RUNNER" "$bam" "$dataset_id" "$RUN_ID"
}

# Run the smaller datasets first so configuration problems surface early.
run_dataset \
    pacbio-hifi-wgs \
    'PacBio HiFi Revio 48x' \
    'BioProject PRJNA1028149; GIAB HG002' \
    "$DATA_DIR/HG002-PacBio-HiFi-Revio-GRCh38/HG002_PacBio-HiFi-Revio_20231031_48x_GRCh38-GIABv3.bam"

run_dataset \
    illumina-wgs \
    'Illumina HiSeq 2x250 Novoalign' \
    'GIAB HG002 NIST Illumina 2x250' \
    "$DATA_DIR/HG002-Illumina-2x250-GRCh38/HG002.GRCh38.2x250.bam"

run_dataset \
    ont-wgs \
    'Oxford Nanopore R10.4.1 SUP' \
    'ONT Open Data GIAB HG002 2023.05' \
    "$DATA_DIR/HG002-ONT-SUP-GRCh38/hg002.haplotagged.bam"

printf 'All paper benchmarks complete. Aggregated results: %s/output/%s\n' \
    "$SCRIPT_DIR" "$RUN_ID"
