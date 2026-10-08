#!/usr/bin/env bash
# Sync the working tree to the build server (tar over ssh).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/remote-lib.sh"
remote_sync
echo "sync complete: $REMOTE_HOST:$REMOTE_SRC" >&2
