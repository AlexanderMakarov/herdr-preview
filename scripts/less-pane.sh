#!/usr/bin/env bash
# Pane `less`: overlay above the current app when file-viewer is unavailable.
set -eu

file="${HERDR_PREVIEW_LESS_FILE:?HERDR_PREVIEW_LESS_FILE missing}"
line="${HERDR_PREVIEW_LESS_LINE:-}"

if [ -n "$line" ]; then
  exec less "+${line}" -- "$file"
else
  exec less -- "$file"
fi
