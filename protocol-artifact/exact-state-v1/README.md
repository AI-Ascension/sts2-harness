# `exact-state-v1` protocol artifact (consumed copy)

This directory is the harness-consumed copy of the `sts2-protocol/exact-state-v1` release-like
artifact at protocol commit `a240af7` (schema digest
`3777ce521bbee30c68b9e25ee188e880cec22f7cdb15350936093b02c2471ab9`). It carries the exact-state,
checkpoint-manifest, and coverage-contract envelope schemas, one synthetic end-to-end golden with
its canonical bytes and computed identifiers, and harness-local checksums.

The harness consumes only this copied artifact; it never imports protocol implementation internals
and owns no game state. The copied envelopes are inert: schema validity does not prove coverage,
restorability, or integrity, and a matching exact-state digest supports a same-start claim only
with complete declared coverage, enforced compatibility, verified restore, and controlled external
inputs. The golden payload is a synthetic fixture, not a game capture.

`crates/harness/src/execution/exact_checkpoint.rs` is the local consumer mapping: it validates the
identifier namespaces and stores exact payloads, manifests, and blobs by content digest under a
configured root. `crates/harness/tests/exact_checkpoint_artifact.rs` binds that mapping to this
artifact. Engine capture/restore and the live supported-boundary matrix remain open gates.
