#!/usr/bin/env bash
# Run a bash command in the Linux probe container from a macOS or Linux host.
# Build the image once:  docker build -t eostage-linux-probe scripts/linux
# Usage:                 bash scripts/linux/run_probe.sh <worktree> <out_dir> "<bash command run in /src>"
# Full Linux check:      bash scripts/linux/run_probe.sh . /tmp/eostage-linux-out "bash scripts/linux/full_test.sh"
set -euo pipefail

if [ "$#" -lt 3 ]; then
    echo "Usage: $0 <worktree> <out_dir> <cmd>" >&2
    exit 2
fi

WT="$(cd "$1" && pwd)"
mkdir -p "$2"
OUT_DIR="$(cd "$2" && pwd)"
CMD="$3"

SUFFIX="-$(basename "$WT")"
PREP="mkdir -p /src && tar -C /wt --exclude=./target --exclude=./ui/node_modules --exclude=./.git -cf - . | tar -C /src -xf - && cd /src && export TMPDIR=/work HDF5_PLUGIN_PRELOAD='::' HDF5_USE_FILE_LOCKING='FALSE' CARGO_TERM_COLOR=never && "

exec docker run --rm \
    -v "${WT}:/wt:ro" \
    -v "${OUT_DIR}:/out" \
    -v "eostage-cargo-registry:/root/.cargo/registry" \
    -v "eostage-target${SUFFIX}:/src/target" \
    -v "eostage-work${SUFFIX}:/work" \
    -v "eostage-node-modules${SUFFIX}:/src/ui/node_modules" \
    eostage-linux-probe \
    bash -c "${PREP}${CMD}"
