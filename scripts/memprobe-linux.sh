#!/usr/bin/env bash
# Run the memory probe under Linux/glibc with a hard memory ceiling.
#
#   scripts/memprobe-linux.sh <data-dir> <out-dir> <memprobe args...>
#
# <data-dir> is mounted read-only at /data and <out-dir> read-write at /out, so
# probe arguments name paths under those. Everything after them goes to the
# probe unchanged.
#
#   scripts/memprobe-linux.sh ~/github/frehoy/karta/data/current /tmp/probe \
#     build-model sweden /out/model.ferrobus
#
# The ceiling is the point: an over-budget run dies as a container, log intact,
# rather than taking the VM down with it.
set -euo pipefail

cd "$(dirname "$0")/.."

DATA_DIR="${1:?usage: memprobe-linux.sh <data-dir> <out-dir> <memprobe args...>}"
OUT_DIR="${2:?usage: memprobe-linux.sh <data-dir> <out-dir> <memprobe args...>}"
shift 2

MEMORY="${MEMPROBE_MEMORY:-14g}"
IMAGE="${MEMPROBE_IMAGE:-ferrobus-memprobe}"

mkdir -p "$OUT_DIR"

echo "==> building $IMAGE"
docker build -q -f docker/memprobe.Dockerfile -t "$IMAGE" .

echo "==> running with --memory=$MEMORY (swap disabled)"
# --memory-swap equal to --memory turns swap off, so the peak is real RSS.
exec docker run --rm \
  --memory="$MEMORY" --memory-swap="$MEMORY" \
  -v "$(cd "$DATA_DIR" && pwd):/data:ro" \
  -v "$(cd "$OUT_DIR" && pwd):/out" \
  -e FERROBUS_DATE -e FERROBUS_OSM -e FERROBUS_GTFS \
  -e FERROBUS_MAX_TRANSFER_TIME -e FERROBUS_RSS_SAMPLE_SECS \
  -e MALLOC_ARENA_MAX -e RUST_LOG \
  "$IMAGE" "$@"
