# Bounded map evaluation

`sts2-map-evaluation-matrix` runs the harness-owned, offline four-mode map
evaluation against one deterministic synthetic visible-map snapshot. It derives
the graph, analysis, candidate action, and independent-best action from that
snapshot; the fixture does not provide an expected route to the evaluator.

With the pinned Linux renderer available, run:

```bash
STS2_MAP_RENDERER_BINARY=/absolute/path/map-visualizer \
STS2_MAP_RENDERER_SHA256=782d0d24d795a35335b84dcb8b08458af5e97eeca71b853914cc8b590ad1462f \
cargo run --locked --offline --bin sts2-map-evaluation-matrix
```

The executable prints one JSON report. Its `rows` contain `next_move_only`,
`graph`, `graph_analysis`, and `graph_analysis_image` in order. Each row records
request, graph, analysis, and image byte counts; latency; topology and proposal
errors; snapshot and analysis digests; and the selected and independent-best
action IDs. The image row is `available` only after the configured renderer is
hash-verified, invoked in a fresh temporary directory, and its PNG and manifest
are checked. `complete` is true only when all four rows are present and that
image row has nonzero image bytes; otherwise `incomplete_reasons` explains the
missing renderer evidence.

The synthetic limits are finite: at most 64 tasks, 256 decisions, 256 graph
nodes, 1,024 edges, 256 candidate routes, 256 route nodes, 256 KiB per
serialized request, and 120 seconds per measured operation. The matrix is
bounded offline evidence for serialization, topology, analysis, and renderer
integration. It does not measure model comprehension, provider quality, player
outcomes, live win rate, game-rule correctness, or a live host campaign.
