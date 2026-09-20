#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

# Linux CI/local entrypoint. Build the actual bridge; only its transport is synthetic.
# The enclosing CI job bounds the build. The integration suite bounds its child processes.
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf '%s\n' 'compiled-ci accepts no arguments' >&2
  exit 2
fi
root="$(git rev-parse --show-toplevel)"
if [[ "$root" != /* ]]; then
  printf '%s\n' 'compiled-ci requires an absolute checkout root' >&2
  exit 2
fi
cd "$root"
revision="$(git rev-parse --verify HEAD)"
if [[ ! "$revision" =~ ^[0-9a-f]{40}$ ]]; then
  printf '%s\n' 'compiled-ci requires a full source revision' >&2
  exit 2
fi

# Do not reuse an ambient binary locator or shared Cargo output directory.
export CARGO_TARGET_DIR="$root/target/jev-compiled-ci"
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
cargo +1.97.1 build --locked --package sts2-harness --bin sts2-jev-bridge
binary="$CARGO_TARGET_DIR/debug/sts2-jev-bridge"
test -f "$binary"
test -x "$binary"
STS2_JEV_TEST_BRIDGE="$(realpath -- "$binary")"
export STS2_JEV_TEST_BRIDGE
export STS2_JEV_TEST_SOURCE_REVISION="$revision"
node --test experiments/jev-evaluation/compiled-bridge.integration.mjs

# Detect an ordinary concurrent checkout move; this is not a clean-tree/build attestation.
test "$(git rev-parse --verify HEAD)" = "$revision"
