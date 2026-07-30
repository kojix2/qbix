#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
DATA_DIR=${QBIX_DATA_DIR:-"$SCRIPT_DIR/data"}

usage() {
  cat <<'EOF'
Usage: ./download_real_data.sh [pacbio] [ont] [illumina]

Download the paper benchmark BAMs and their indexes. With no dataset names,
all three are downloaded. aria2c is used when available; otherwise wget -c is
used. Existing complete files are verified and skipped.

Environment:
  QBIX_DATA_DIR  destination directory (default: paper/work/data)
EOF
}

if command -v aria2c >/dev/null 2>&1; then
  DOWNLOADER=aria2c
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER=wget
else
  echo "error: aria2c or wget is required" >&2
  exit 1
fi

verify_file() {
  local path=$1
  local expected_bytes=$2
  local expected_md5=$3
  local actual_bytes

  actual_bytes=$(stat -c '%s' "$path")
  if [[ "$actual_bytes" != "$expected_bytes" ]]; then
    echo "error: $path is $actual_bytes bytes; expected $expected_bytes" >&2
    return 1
  fi

  if [[ -n "$expected_md5" ]]; then
    printf '%s  %s\n' "$expected_md5" "$path" | md5sum --check --status
  fi
}

download_file() {
  local directory=$1
  local url=$2
  local filename=$3
  local expected_bytes=$4
  local expected_md5=${5:-}
  local destination="$DATA_DIR/$directory"
  local path="$destination/$filename"

  mkdir -p "$destination"
  if [[ -f "$path" ]] && verify_file "$path" "$expected_bytes" "$expected_md5"; then
    echo "Verified, skipping: $path"
    return
  fi

  echo "Downloading: $url"
  if [[ "$DOWNLOADER" == aria2c ]]; then
    aria2c \
      --continue=true \
      --max-connection-per-server=8 \
      --split=8 \
      --min-split-size=16M \
      --file-allocation=none \
      --auto-file-renaming=false \
      --remote-time=true \
      --dir="$destination" \
      --out="$filename" \
      "$url"
  else
    wget --continue --output-document="$path" "$url"
  fi

  verify_file "$path" "$expected_bytes" "$expected_md5"
  echo "Verified: $path"
}

download_pacbio() {
  local base=https://ftp.ncbi.nlm.nih.gov/ReferenceSamples/giab/data/AshkenazimTrio/HG002_NA24385_son/PacBio_HiFi-Revio_20231031
  local bam=HG002_PacBio-HiFi-Revio_20231031_48x_GRCh38-GIABv3.bam
  download_file HG002-PacBio-HiFi-Revio-GRCh38 "$base/$bam" "$bam" \
    74680242384 72721742ea4a90a8301e1086496d82c1
  download_file HG002-PacBio-HiFi-Revio-GRCh38 "$base/$bam.bai" "$bam.bai" \
    23424832 9d0218df76ab404cf01e87b453f19458
}

download_ont() {
  local base=https://ont-open-data.s3.amazonaws.com/giab_2023.05/analysis/variant_calling/hg002_sup_all
  local bam=hg002.haplotagged.bam
  download_file HG002-ONT-SUP-GRCh38 "$base/$bam" "$bam" 311143990628
  download_file HG002-ONT-SUP-GRCh38 "$base/$bam.bai" "$bam.bai" 93615328
}

download_illumina() {
  local base=https://ftp.ncbi.nlm.nih.gov/ReferenceSamples/giab/data/AshkenazimTrio/HG002_NA24385_son/NIST_Illumina_2x250bps/novoalign_bams
  local bam=HG002.GRCh38.2x250.bam
  download_file HG002-Illumina-2x250-GRCh38 "$base/$bam" "$bam" \
    130770531934 56c30eaa4e2f25ff0ac80ef30e09d78e
  download_file HG002-Illumina-2x250-GRCh38 "$base/$bam.bai" "$bam.bai" \
    9451360 a3c2b449df6509ca83fbd3fea22b9aee
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  usage
  exit 0
fi

datasets=("$@")
if [[ ${#datasets[@]} -eq 0 ]]; then
  datasets=(pacbio ont illumina)
fi

echo "Downloader: $DOWNLOADER"
echo "Destination: $DATA_DIR"
for dataset in "${datasets[@]}"; do
  case "$dataset" in
    pacbio) download_pacbio ;;
    ont) download_ont ;;
    illumina) download_illumina ;;
    all)
      download_pacbio
      download_ont
      download_illumina
      ;;
    *)
      echo "error: unknown dataset '$dataset'" >&2
      usage >&2
      exit 2
      ;;
  esac
done

echo "All requested downloads are complete."
