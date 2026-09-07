# Dense synthetic map view bundle fixture

This separate demo fixture is generated through
`MapViewBundle::from_runtime_snapshot` from the checked in test snapshot. It
contains 15 floors, five lanes, crossed branch and merge edges, visited past
nodes, a current node, an unreachable visible node, multiple rest, shop, and
elite rooms, and five visible terminals. It is a harness generated demo and is
kept separate from the finalized protocol golden fixture.

Verify the exact snapshot, analysis, and manifest bytes with:

```text
CARGO_TARGET_DIR=/tmp/ascension-map-targets/harness cargo test --locked --package sts2-harness --test map_bundle_demo_fixture
```
