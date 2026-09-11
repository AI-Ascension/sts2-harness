# Synthetic map view bundle fixture

This fixture is generated from the finalized `runtime-map-v1` protocol golden. It is an offline, deterministic harness artifact: `renderer_version` is `unrendered`, `viewer.json` is the exact `{}` sentinel, and `decision.json` is marked non-dispatchable. The feed timestamp is fixed metadata and is not part of the bundle digest. It carries no game, provider, or host authority.

The checked-in bytes are verified by the public harness APIs in
`map_bundle_fixture.rs`. Re-run the byte-for-byte regeneration assertion with:

```text
CARGO_TARGET_DIR=/tmp/ascension-map-targets/harness cargo test --locked --package sts2-harness --test map_bundle_fixture
```

That test adapts `visible-map.json`, calls `MapAnalysis::analyze`, rebuilds the
unrendered `MapViewBundle`, and compares both payload bytes and the manifest
digest to this fixture.
