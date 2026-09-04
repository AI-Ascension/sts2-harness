# Exo-backed STS2 agent adapter

This experiment is the harness-owned configuration and adapter seam for a reviewed Exo
deployment. It does not contain an Exo checkout, model weights, credentials, game files, saves, or
provider output. The checked-in Rust adapter accepts only a sanitized fair-play projection and the
complete host-generated semantic action-ID set.

## Configuration

Copy `config.example.toml` to an operator-owned location and fill in a reviewed Exo revision and
endpoint outside the repository. Do not commit the copy. `revision` is mandatory; an empty or
floating revision is rejected by `ExoConfig`.

The transport implementation supplied by the operator must enforce the requested timeout and
response limit. The harness checks the returned byte count again, rejects malformed structured
decisions, and treats unavailable or timed-out Exo calls as fail-closed provider outcomes.

## Data boundary

Exo may receive ordinary player-visible state, explicitly labeled derived facts, the current
generation, and the current host-generated action IDs. It must not receive raw host objects,
executables, PCK/DLL bytes, saves, credentials, private prompts, screen coordinates, input events,
future RNG state, or unrevealed random outcomes. Model responses are parsed into a small decision
enum; verbatim output is not a trajectory artifact.

Live Exo connectivity, the selected revision, licensed STS2 build, and gameplay compatibility are
`unverified` until a separately recorded runtime handoff supplies exact build/configuration
lineage.
