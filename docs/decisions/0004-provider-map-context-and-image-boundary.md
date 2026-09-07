# ADR 0004: Provider map context and image boundary

Status: implemented against the protocol-owned `runtime-map-v1` and
`visible-map-v1` contract at revision
`d9ffb190ad8990e15f43d7992581dcb2d60b1971`. Companion changes are unmerged;
final assembly and live host/provider acceptance remain separate gates.

## Decision

The harness keeps the existing `sts2.exo-decision-v1` request and the legacy
fair-play observation unchanged. A current map decision opts into the additive
`sts2.exo-decision-map-v1` request profile. Its `map_context` contains the
protocol snapshot identity, the complete node and edge lists, the current
generation-bound node/action bindings, the explicit `fair-play-v1` version,
the negotiated `runtime-map-v1` profile, and the deterministic harness map
analysis when available.

Map graph identifiers are stable snapshot identities. They never become
dispatchable action identifiers. Before a map context is attached, the
harness compares source state ID, generation, and the exact unique set of current
action IDs with the fresh `sts2.map_snapshot` response. A permutation is valid;
duplicate, missing, extra, or stale bindings fail closed. The runtime boundary
must also match each binding's complete action payload to the current catalog;
matching an opaque action ID alone does not validate its map destination.
On a mismatch the
runner must reacquire state and catalog. A `next-move-only` runtime mode is an
explicit labeled fallback; it is not reported as complete-map support.

PNG is an optional additive attachment. It is validated as a complete PNG,
bounded to 8 MiB and 4096 by 4096 pixels, and bound to the snapshot digest and
its own SHA-256 digest. The independent map request budget is 8 MiB; the
legacy 128 KiB request limit remains unchanged for `sts2.exo-decision-v1`.
The Astra bridge receives validated bytes through a temporary file and the
installed `codex exec --image FILE` option. It removes binary bytes from the
text prompt while retaining image metadata and the full graph.

The runtime may invoke the configured visualizer with:

```text
map-visualizer render --bundle <input-dir> --out <fresh-output-dir>
```

It accepts an image only after matching the output manifest's snapshot,
analysis, renderer, and PNG digests. Missing optional rendering leaves a
graph-only context. `graph-image` mode fails explicitly when a verified image
cannot be produced.

## Consequences

The provider path has a typed capability and identity fence without widening
the legacy observation allowlist. Provider capture tests can inspect the
serialized additive request, including every node, edge, binding, and image
digest. Actual provider acceptance remains distinct from model image
comprehension and from host action settlement.

The protocol dependency is pinned to the supplied frozen contract rather than a
sibling checkout. Its map schema digest is
`6340f3cbe6c1b5728144fe89fdfdf8645acf2f59a77c0e0c30ebfeafc77515d8`.
The exact compiled adapter, provider CLI capture, and final host/provider slice
must agree on that contract before cross-repository integration is accepted.
