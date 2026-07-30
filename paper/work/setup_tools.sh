#!/usr/bin/env bash
# Build/fetch the comparison tools (bri, atlantool) used by the paper benchmark.
# Pinned to specific commits/releases so results are reproducible.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ENV_DIR="$SCRIPT_DIR/.pixi/envs/default"
TOOLS_DIR="$SCRIPT_DIR/tools"
BRI_COMMIT=6004b6a3ce16b459ec135cc2798308ff7910bc3e
ATLANTOOL_RELEASE_TAG=release-983975f
ATLANTOOL_ASSET_URL="https://github.com/VCCRI/atlantool/releases/download/${ATLANTOOL_RELEASE_TAG}/atlantool-linux"
ATLANTOOL_SHA256=aed7a8530198d5302a899ea645be9207ae735828080470ed35ce31a7f4f5b580

mkdir -p "$TOOLS_DIR"

echo "Building bri @ $BRI_COMMIT"
rm -rf "$TOOLS_DIR/bri-src"
git clone https://github.com/jts/bri.git "$TOOLS_DIR/bri-src"
git -C "$TOOLS_DIR/bri-src" checkout --quiet "$BRI_COMMIT"
make -C "$TOOLS_DIR/bri-src" \
    CFLAGS="-O3 -std=c99 -fsigned-char -D_FILE_OFFSET_BITS=64 -g -I$ENV_DIR/include -L$ENV_DIR/lib -Wl,-rpath,$ENV_DIR/lib"
cp "$TOOLS_DIR/bri-src/bri" "$TOOLS_DIR/bri"

echo "Fetching atlantool @ $ATLANTOOL_RELEASE_TAG"
curl -sL --max-time 120 -o "$TOOLS_DIR/atlantool-linux" "$ATLANTOOL_ASSET_URL"
chmod +x "$TOOLS_DIR/atlantool-linux"
actual_sha256=$(sha256sum "$TOOLS_DIR/atlantool-linux" | cut -d' ' -f1)
if [[ "$actual_sha256" != "$ATLANTOOL_SHA256" ]]; then
    echo "error: atlantool-linux sha256 mismatch: expected $ATLANTOOL_SHA256, got $actual_sha256" >&2
    exit 1
fi

cat > "$TOOLS_DIR/versions.json" <<JSON
{
  "bri": {
    "repo": "https://github.com/jts/bri",
    "commit": "$BRI_COMMIT",
    "version_output": "$("$TOOLS_DIR/bri" version)"
  },
  "atlantool": {
    "release_tag": "$ATLANTOOL_RELEASE_TAG",
    "asset_url": "$ATLANTOOL_ASSET_URL",
    "sha256": "$ATLANTOOL_SHA256"
  }
}
JSON

echo "Tools ready: $TOOLS_DIR/bri, $TOOLS_DIR/atlantool-linux"
