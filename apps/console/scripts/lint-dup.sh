#!/usr/bin/env bash
# Copy-paste gate (jscpd, config in .jscpd.json).
# jscpd's internal packages don't resolve under bun's isolated install, so it
# lives in its own npm-installed dir, created once and reused.
set -euo pipefail
cd "$(dirname "$0")/.."
dir=.cache/jscpd
if [ ! -x "$dir/node_modules/.bin/jscpd" ]; then
  mkdir -p "$dir"
  npm install --silent --no-audit --no-fund --prefix "$dir" jscpd@4.0.5 >/dev/null
fi
exec "$dir/node_modules/.bin/jscpd" --config .jscpd.json "$@"
