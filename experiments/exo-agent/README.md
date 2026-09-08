# Exo-backed STS2 agent adapter

This experiment is the harness-owned configuration and adapter seam for a reviewed Exo
deployment. It does not contain an Exo checkout, model weights, credentials, game files, saves, or
provider output. The checked-in Rust adapter accepts only a sanitized fair-play projection and the
complete host-generated semantic action-ID set. Map context is a separate, opt-in projection for
Map-stage decisions; it never becomes a second action authority.

## Configuration

Copy `config.example.toml` to an operator-owned location and set an exact reviewed Exo revision and
endpoint outside the repository. Do not commit the copy. The checked-in revision is the public
audit revision reviewed on 2026-09-02; a deployment using another revision must replace it with a
separately reviewed 40- or 64-character lowercase commit hash. Empty, floating, placeholder, and
all-zero revisions are rejected by `ExoConfig`.

The harness supplies `ExoProcessTransport` for an operator-owned bridge when a direct process is
appropriate. It passes configured arguments directly, clears the environment except for an
explicit safe-name allowlist, writes one request to stdin, bounds stdout, enforces a timeout, and
never invokes a shell. A custom `ExoTransport` remains possible for another reviewed boundary. The
harness checks the returned byte count again, rejects malformed structured decisions, and treats
unavailable or timed-out Exo calls as fail-closed provider outcomes.

### Optional map context

The runtime coordinator can request a host-authored map projection at Map stages under either the
`runtime-v3-gameplay` or `runtime-v4-expert` profile. Keep the ordinary behavior by leaving
`STS2_ENABLE_MAP_CONTEXT` unset or setting it to `false`. To enable the map path, set the exact
boolean and use a bridge that understands the map request schema:

```text
STS2_RUNTIME_PROFILE=runtime-v3-gameplay
STS2_ENABLE_MAP_CONTEXT=true
STS2_EXO_MAX_REQUEST_BYTES=393443
```

The flag accepts only `true` or `false`. With the flag enabled, `STS2_EXO_MAX_REQUEST_BYTES` defaults
to `393443` and must remain exactly that value. This bound carries the ordinary 128 KiB request, one
complete 256 KiB map snapshot, and the fixed map wrapper. The snapshot limit is body-only: raw native,
gateway, and projected-MCP whole envelopes remain bounded at 256 KiB, so a snapshot at its own limit
may not fit once those wrappers are added. Ordinary MCP stdout remains bounded at 256 KiB; only the
map-profile framed stdout allowance is 512 KiB for the escaped snapshot and JSON-RPC/content wrapper.
Provider responses remain bounded at 8 KiB.
The bridge must accept `schema: "sts2.exo-decision-map-v1"` and validate the `map_context` object;
an ordinary-schema-only bridge cannot serve map-stage requests. The map schema is used only at Map
stages, while other stages continue to use `sts2.exo-decision-v1`.

After the current observation and legal-action catalog are read, the runner starts a short-lived
`runtime-map-v1` MCP profile and calls `sts2.map_snapshot`. The profile has an exact seven-tool
catalog, including the six gameplay tools plus the map reader. The runner accepts only a response
whose projection is `available`, `complete`, and `current`, whose state ID and generation match the
observation, and whose bindings exactly match the current host-generated `select_map_node` action
IDs. The snapshot uses `visible-map-v1`, the schema digest
`ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b`, and a canonical SHA-256
snapshot digest. It is bounded at 256 KiB, 256 nodes, 1,024 edges, and 256 bindings; its graph must
be acyclic. Lower-level whole-envelope checks remain authoritative even with the wider map-profile
framed stdout allowance. The snapshot is carried as escaped JSON text inside the wrapper.

If the map profile, snapshot, digest, identity, generation, or action bindings are unavailable or
invalid, the episode fails closed. Native target support for either profile plus map context remains
unverified. The runner does not send an ordinary request for that Map stage and does not invent a
fallback action. The projection remains subject to the fair-play boundary:
raw host objects, assemblies, saves, credentials, private prompts, hidden RNG internals, and
unrevealed random outcomes are not sent to Exo. Target-build map production, live wiring, provider
interpretation, and gameplay compatibility require a separately recorded runtime handoff.

## Data boundary

Exo may receive ordinary player-visible state, explicitly labeled derived facts, the current
generation, and the current host-generated action IDs. It must not receive raw host objects,
executables, PCK/DLL bytes, saves, credentials, private prompts, screen coordinates, input events,
hidden RNG internals, or unrevealed random outcomes. The host's `visible_seed` is preserved by
default for repeatable invocation and replay, as required by the owner. An explicitly seed-blind
experiment can set `STS2_EXO_FORWARD_VISIBLE_SEED=false`. Model responses are parsed
into a small decision enum; verbatim output is not a trajectory artifact.

Live Exo connectivity, the selected revision, licensed STS2 build, and gameplay compatibility are
`unverified` until a separately recorded runtime handoff supplies exact build/configuration
lineage.
