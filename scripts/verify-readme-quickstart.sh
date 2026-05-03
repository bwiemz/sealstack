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

# Print the lines between the first pair of triple-backtick fences in $1.
# Excludes the fences themselves.
extract_first_code_block() {
    local path="$1"
    awk '/^```/{n+=1} n==1 && !/^```/' "$path"
}

check() {
    local readme="$1" example="$2"
    local extracted
    extracted="$(extract_first_code_block "${readme}")"
    if ! diff <(printf '%s\n' "${extracted}") "${example}" > /dev/null; then
        echo "::error::${readme} Quickstart code block does not match ${example}" >&2
        diff <(printf '%s\n' "${extracted}") "${example}" || true
        exit 1
    fi
}

check sdks/typescript/README.md sdks/typescript/examples/quickstart.ts
check sdks/python/README.md     sdks/python/examples/quickstart.py
echo "READMEs match examples byte-for-byte"
