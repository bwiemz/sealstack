#!/usr/bin/env bash
# Verify each SDK README's first fenced code block matches its
# examples/quickstart.{ts,py} byte-for-byte.
#
# Catches the common drift pattern where a contributor updates the
# README and forgets to update the runnable example (or vice versa).
# Per the v0.3 SDK contract spec §15.3.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

tmp_extracted="$(mktemp)"
trap 'rm -f "${tmp_extracted}"' EXIT

# Write the lines between the first pair of triple-backtick fences in
# $1 to $2 (excluding the fences themselves). Routing through a temp
# file avoids `$()` command-substitution stripping trailing blank lines,
# which would break byte-equality silently if either file ever picked
# up trailing whitespace.
extract_first_code_block_to() {
    local path="$1" out="$2"
    awk '/^```/{n+=1; next} n==1 {print}' "$path" > "$out"
}

check() {
    local readme="$1" example="$2"
    extract_first_code_block_to "${readme}" "${tmp_extracted}"
    if ! diff -u "${tmp_extracted}" "${example}" > /dev/null; then
        echo "::error::${readme} Quickstart code block does not match ${example}" >&2
        diff -u "${tmp_extracted}" "${example}" || true
        exit 1
    fi
}

check sdks/typescript/README.md sdks/typescript/examples/quickstart.ts
check sdks/python/README.md     sdks/python/examples/quickstart.py
echo "READMEs match examples byte-for-byte"
