# `exact-checkpoint-reference-v1` protocol artifact (consumed copy)

Harness-consumed copy of the `sts2-protocol/exact-checkpoint-reference-v1` release-like artifact
(schema digest `028e00d06f9f2b16cb9097f47aedd057e74046a7cb2ba97362978e18029f48ab`). It carries the
closed, digest-free public reference envelope, its goldens, and harness-local checksums.

`crates/harness/src/checkpoint_projection.rs` produces this envelope from a keyed projection, and
`crates/harness/tests/exact_checkpoint_reference_conformance.rs` validates a produced summary against
this copied schema. A valid reference proves nothing about coverage, restorability, or honest handle
issuance.
