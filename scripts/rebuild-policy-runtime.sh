#!/usr/bin/env bash
# Rebuild the WASM policy runtime asset that sealstack-csl ships as a fixture.
#
# Requires: `rustup target add wasm32-unknown-unknown`.
# CI runs this and fails if the committed asset differs from the rebuilt output.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

cargo build \
  -p sealstack-policy-runtime \
  --target wasm32-unknown-unknown \
  --release

src="target/wasm32-unknown-unknown/release/sealstack_policy_runtime.wasm"
dst="crates/sealstack-csl/assets/policy_runtime.wasm"

mkdir -p "$(dirname "${dst}")"
cp "${src}" "${dst}"

size=$(wc -c < "${dst}")
echo "wrote ${dst} (${size} bytes)"
