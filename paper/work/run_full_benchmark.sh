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
    --profile full
    --platform "${QBIX_DATASET_PLATFORM:-unspecified}"
    --source "${QBIX_DATASET_SOURCE:-unspecified}"
    --region "${QBIX_DATASET_REGION:-whole-genome}"
    --storage "${QBIX_STORAGE:-local-storage-unspecified}"
    --max-index-s "${QBIX_MAX_INDEX_S:-7200}"
    --max-scan-s "${QBIX_MAX_SCAN_S:-1800}"
)

for stage in preflight prepare index check queries summary; do
    "$PYTHON" "$SCRIPT_DIR/benchmark.py" "$stage" "$BAM" "${COMMON[@]}"
done

printf 'Full benchmark complete. Results: %s/output/%s/%s\n' \
    "$SCRIPT_DIR" "$RUN_ID" "$DATASET_ID"
