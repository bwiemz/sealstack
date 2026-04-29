#!/usr/bin/env bash
# Rebuild the WASM policy runtime asset that sealstack-csl ships as a fixture.
#
# Requires: `rustup target add wasm32-unknown-unknown`.
# CI runs this and fails if the committed asset differs from the rebuilt output.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

# sealstack-policy-runtime is `[workspace] exclude`'d from the root workspace,
# so `-p` lookup from the workspace root won't find it. Use --manifest-path
# to invoke cargo on the standalone crate.
cargo build \
  --manifest-path crates/sealstack-policy-runtime/Cargo.toml \
  --target wasm32-unknown-unknown \
  --release

# Output lives next to the standalone crate's target dir, not the workspace's.
src="crates/sealstack-policy-runtime/target/wasm32-unknown-unknown/release/sealstack_policy_runtime.wasm"
dst="crates/sealstack-csl/assets/policy_runtime.wasm"

mkdir -p "$(dirname "${dst}")"
cp "${src}" "${dst}"

size=$(wc -c < "${dst}")
echo "wrote ${dst} (${size} bytes)"
