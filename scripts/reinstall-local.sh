#!/usr/bin/env bash
# Build release binary and refresh herdr plugin link (link does not run [[build]]).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo build --release
herdr plugin link "$ROOT"
