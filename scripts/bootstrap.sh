#!/usr/bin/env bash
set -euo pipefail

echo "Checking prerequisites..."
command -v rustc >/dev/null || { echo "missing: rustc"; exit 1; }
command -v cargo >/dev/null || { echo "missing: cargo"; exit 1; }
command -v node  >/dev/null || { echo "missing: node";  exit 1; }
command -v pnpm  >/dev/null || { echo "missing: pnpm";  exit 1; }
command -v docker >/dev/null || { echo "missing: docker"; exit 1; }

echo "Installing JS deps..."
pnpm install

echo "Cargo check..."
cargo check --workspace

echo "Bootstrap complete."
