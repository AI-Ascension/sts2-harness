# Harness map analysis API handoff

Status: proposed harness consumer API, pending the protocol owner's frozen
`runtime-map-v1` / `visible-map-v1` types. This document does not define or
replace the upstream snapshot schema. The harness accepts an adapter view of a
validated snapshot and derives the types below.

## Input seam

The pure analyzer consumes a `ValidatedMapGraph` adapter containing:

* `snapshot_digest`, `map_instance`, `act`, `source_state_id`, `generation`,
  and `complete`/`freshness` evidence. The finalized protocol adapter accepts
  `map_instance_id`/`act_id`/`state_id`, `position`, `bindings`, and
  `terminal_node_ids` from `runtime-map-v1`; serialized action option IDs
  remain independent from graph and host action IDs;
* stable opaque node IDs, bounded logical row/column coordinates, public room
  category/status, and directed edges whose endpoints are in the node set;
* the current node (or pre-start), exact legal destination bindings
  `(node_id, action_id)`, and explicitly identified visible terminal nodes.

This is an internal validated view. Protocol consumers will provide the
adapter from the accepted `VisibleMapSnapshot`; the harness does not own the
wire schema, host extraction, legal-action authority, or mutation operation.

## `MapAnalysis` serialized fields

The deterministic payload is versioned as `sts2.map-analysis-v1` and contains:

```text
snapshot_digest: string
analysis_version: string
evaluator_version: string
map_instance: string
act: string
source_state_id: string
generation: u64
assumptions: [string]
completeness: "complete" | "incomplete" | "unavailable"
approximation: "exact" | "bounded_candidates" | "incomplete_input"
topology: { node_count, edge_count, current_node, reachable_nodes,
            branch_nodes, merge_nodes, cyclic: bool }
node_metrics: [{ node_id, reachable, distance_from_current,
                 distance_to_terminal, category_distances, ancestors,
                 descendants }]
terminal_counts: [{ terminal_id, count_decimal, status: "exact" | "overflow" | "incomplete" }]
legal_destination_counts: [{ destination_node_id, action_id, count_decimal,
                              status: "exact" | "overflow" | "incomplete" }]
candidate_routes: [{ nodes, first_action_id,
                     score: { distance_to_terminal, category_score,
                              rest_count, shop_count, elite_count,
                              elite_exposure_before_rest,
                              retained_branching, route_length },
                     tie_break_key,
                     selection: "exact" | "bounded_candidates" | "incomplete_input",
                     assumptions }]
warnings: [string]
content_digest: string
```

Counts remain decimal strings and never become win/reward probabilities.
Candidate routes are bounded to eight by default and contain concrete node
sequences plus the first current legal action binding. Each score exposes
`rest_count`, `shop_count`, `elite_count`, and
`elite_exposure_before_rest` alongside route distance and retained branching.
`RoutePolicy` has public category weights plus explicit rest, shop, elite, and
elite-before-rest weights; all default to zero, and weighted scores remain
structural preferences rather than reward estimates. A cycle or malformed
graph is a typed analysis error; it is never handled as a DAG and never gets a
fabricated route count.

## `MapViewBundle` serialized fields

The bundle has a deterministic manifest/payload and separate operation
metadata. The manifest binds:

```text
bundle_version: string
snapshot_digest: string
analysis_digest: string
map_instance: string
act: string
run_id: string
episode_id: string
trajectory_id: string
model_execution_id: string | null
schema_profile: string
schema_digest: string
analysis_version: string
renderer_version: string
presentation: { width, height, layout_version } | null
origin: { owner, source, generator, license }
contents: { snapshot_ref, analysis_ref, svg_digest | null, png_digest | null }
history: { source_state_id, generation, action_catalog_digest }
```

The payload references or embeds the exact snapshot and analysis bytes and may
carry deterministic SVG/PNG artifacts once a renderer attaches them. Atomic
publication writes one manifest only after every referenced digest is
verified. The filesystem store serializes the immutable-directory and feed
transaction with a bounded OS-backed exclusive lock at the store root. The
lock file is persistent and is never removed as stale state; the operating
system releases the lock when a writer exits, including after process death.
Each store retains at most 4096 immutable digest directories and rejects a new
digest at that finite capacity rather than deleting historical bundles. Root
directory scans are bounded, and publication/feed temporary paths use
create-new semantics so a reused operation ID or symlink fails closed and is
left available for diagnosis. A loader rejects malformed JSON, digest
mismatch, missing members, cross-generation bindings, truncated writes, and
unknown required versions.
Historical action bindings are retained for replay inspection but have no
dispatch capability.

## Cache and replay keys

Topology, navigation/legal-overlay, analysis, and render caches have distinct
typed keys. Each includes the relevant projection/profile/version and source
digest; navigation additionally includes generation and catalog digest;
analysis additionally includes public player-state/objective/evaluator
versions; render additionally includes presentation settings. A seed, private
host hash, artifact digest, or cache hit never grants lease or mutation
authority. Cache entries have bounded count/bytes/retention and bounded
in-flight work.

Replay loads the exact validated source-time snapshot bytes, analysis,
manifest lineage, and complete historical action catalog from a bundle or
artifact feed, verifies the bundle before constructing the replay, and
performs no game/provider call. The catalog is reconstructed from every
source snapshot binding rather than bounded candidate routes. Replayed route
intent remains historical and is never dispatched; the fallible replay
constructor exposes corruption instead of creating a partial view.

## Evaluation seam

The synthetic evaluator runs the same controlled graph tasks under four
context modes: `next_move_only`, `graph`, `graph_analysis`, and
`graph_analysis_image`. It reports topology/comprehension errors, missed
route opportunities, invalid proposals, request bytes, and deterministic
latency counters without a paid provider call. It does not infer a win-rate
improvement or model image comprehension from a successful serialization.
