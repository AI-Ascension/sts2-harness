# Patch-diff preparation tool

`sts2-patch-diff` compares two bounded JSON manifest files and emits deterministic JSON containing
the input paths, non-cryptographic FNV-1a fingerprints, quarantine statuses, and changed line
numbers. It is a preparation aid for the M10 release gate; it does not inspect a game installation,
load a managed assembly, promote a patch, or establish runtime compatibility.

Build or run it only when the pinned Rust toolchain is available:

```text
cargo run --manifest-path tools/patch-diff/Cargo.toml -- \
  docs/research/sts2-expert-state-package/data/build-manifest.json \
  candidate-build-manifest.json
```

The canonical manifest is validated against [`patch-manifest.schema.json`](../../patch-manifest.schema.json)
and remains `quarantined` until the licensed target build, exact package hashes, independent leak
checks, full-run/co-op traces, cleanup, replay, rollback, and repository gates are recorded.
