#!/usr/bin/env sh
set -eu

MIMIR_BIN="${MIMIR_BIN:-mimir}"

exec "$MIMIR_BIN" remote drill "$@"
