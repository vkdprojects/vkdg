#!/usr/bin/env bash
# Run the CI gates on the server. Syncs once; each gate runs in the builder
# container. Gates whose tool is missing from the builder image are reported
# as SKIPPED (never silently).
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/remote-lib.sh"

remote_sync

declare -a names=() results=()
failed=0

run_gate() {
  local name="$1"; shift
  echo "=== gate: $name ===" >&2
  # Between gates: if the disk dipped under the guard, wipe the target volume first.
  local free; free="$(remote_free_bytes)"
  if [[ "$free" =~ ^[0-9]+$ ]] && ((free < MIN_FREE_GIB * 1024 * 1024 * 1024)); then
    echo "free space under ${MIN_FREE_GIB} GiB before gate $name" >&2
    remote_wipe_target
  fi
  remote_run "$@"
  local rc=$?
  names+=("$name")
  if ((rc == 0)); then results+=("PASS"); else results+=("FAIL($rc)"); failed=1; fi
}

skip_gate() {
  echo "=== gate: $1 SKIPPED: $2 ===" >&2
  names+=("$1"); results+=("SKIPPED: $2")
}

# Optional tool availability inside the builder image.
has_tool() { remote_run sh -c "command -v $1 >/dev/null 2>&1"; }

run_gate fmt cargo fmt --all --check
run_gate clippy cargo clippy --workspace --all-targets --locked -- -D warnings
run_gate test cargo test --workspace --tests

if has_tool typos; then # Server copy has no .git, so typos would scan the cargo target volume; exclude untracked build dirs.
  run_gate typos typos --exclude target --exclude node_modules .
else skip_gate typos "typos not installed in $BUILDER_IMAGE (cargo install typos-cli failed at image build; rebuild with VKDG_BUILDER_REBUILD=1)"; fi

if has_tool cargo-machete; then run_gate machete cargo machete
else skip_gate machete "cargo-machete not installed in $BUILDER_IMAGE (cargo install failed at image build; rebuild with VKDG_BUILDER_REBUILD=1)"; fi

echo >&2
echo "=== gate summary ===" >&2
for i in "${!names[@]}"; do printf '%-8s %s\n' "${names[$i]}" "${results[$i]}" >&2; done
exit "$failed"
