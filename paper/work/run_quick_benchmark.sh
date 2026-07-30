#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

[[ $# -ge 1 && $# -le 3 ]] || {
    printf 'usage: %s BENCHMARK.bam [DATASET_ID] [RUN_ID]\n' "$0" >&2
    exit 1
}

BAM=$1
if [[ $# -ge 2 ]]; then
    DATASET_ID=$2
else
    DATASET_ID=$(basename -- "$BAM" .bam | sed 's/[^A-Za-z0-9._-]/-/g')
fi
RUN_ID=${3:-$(date -u +%Y%m%dT%H%M%SZ)}
PYTHON=${PYTHON:-python}
COMMON=(
    --dataset-id "$DATASET_ID"
    --run-id "$RUN_ID"
    --max-index-s "${QBIX_MAX_INDEX_S:-60}"
    --max-scan-s "${QBIX_MAX_SCAN_S:-15}"
)

"$PYTHON" "$SCRIPT_DIR/benchmark.py" preflight "$BAM" "${COMMON[@]}"
"$PYTHON" "$SCRIPT_DIR/benchmark.py" prepare "$BAM" "${COMMON[@]}"
"$PYTHON" "$SCRIPT_DIR/benchmark.py" index "$BAM" "${COMMON[@]}"
"$PYTHON" "$SCRIPT_DIR/benchmark.py" check "$BAM" "${COMMON[@]}"
"$PYTHON" "$SCRIPT_DIR/benchmark.py" queries "$BAM" "${COMMON[@]}"
"$PYTHON" "$SCRIPT_DIR/benchmark.py" summary "$BAM" "${COMMON[@]}"

printf 'Benchmark complete. Results: %s/output/%s/%s\n' \
    "$SCRIPT_DIR" "$RUN_ID" "$DATASET_ID"
