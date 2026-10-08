#!/usr/bin/env bash
# Usage: scripts/remote-cargo.sh <cargo args...>
# Syncs the tree, then runs cargo on the server inside vkdg-builder:latest
# (nice 19, serialized via flock /root/vkdg-build/lock). Exit code propagates.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/remote-lib.sh"
(($# > 0)) || die "usage: $0 <cargo args...>"
remote_sync
remote_run cargo "$@"
