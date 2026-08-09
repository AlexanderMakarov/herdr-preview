#!/usr/bin/env bash
# Action `hint`: capture visible snapshot in THIS process, then open the overlay.
# Never call `herdr pane read` from the overlay pane (deadlock).
set -eu

root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
bin="${HERDR_PREVIEW_BIN:-$root/target/release/herdr-preview}"

if [ ! -x "$bin" ]; then
  bin="$root/target/debug/herdr-preview"
fi

exec "$bin" hint
