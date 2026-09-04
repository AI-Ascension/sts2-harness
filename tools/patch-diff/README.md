# Patch-diff preparation tool

`sts2-patch-diff` compares two bounded JSON manifest files and emits deterministic JSON containing
the input paths, non-cryptographic FNV-1a fingerprints, quarantine statuses, and changed line
numbers. It is a preparation aid for the M10 release gate; it does not inspect a game installation,
load a managed assembly, promote a patch, or establish runtime compatibility.

Build or run it only when the pinned Rust toolchain is available:

```text
cargo run --locked --package sts2-patch-diff -- \
  docs/evidence/runtime-v3-preparation/data/build-manifest.json \
  candidate-build-manifest.json
```

Each input is read through a 512-KiB-plus-one-byte limit before JSON parsing. Oversized input,
invalid UTF-8/JSON, and missing, duplicated, or invalid quarantine status fields fail without a
report. The status is read from the root `quarantine.status` field, not a text match. The tool
reports a declared status; it does not authorize that status or validate every manifest field.
The byte limit bounds consumption, not wall time: use ordinary local files, not blocking streams.

CI validates the canonical manifest against the full Draft 2020-12
[`patch-manifest.schema.json`](../../patch-manifest.schema.json), including nested references,
required fields, bounds, and digest patterns, and separately requires `quarantined`:

```text
cargo test --locked --package sts2-patch-diff --test patch_manifest
```

The validator is a pinned test-only dependency with external URL/file resolution features disabled.
Negative tests ensure nested contract violations fail. The tool participates in workspace format,
Clippy, and test gates. The canonical manifest remains `quarantined` until the licensed target
build, exact package hashes, independent leak
checks, full-run/co-op traces, cleanup, replay, rollback, and repository gates are recorded.
