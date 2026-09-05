# Exo-backed STS2 agent adapter

This experiment is the harness-owned configuration and adapter seam for a reviewed Exo
deployment. It does not contain an Exo checkout, model weights, credentials, game files, saves, or
provider output. The checked-in Rust adapter accepts only a sanitized fair-play projection and the
complete host-generated semantic action-ID set.

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

## Data boundary

Exo may receive ordinary player-visible state, explicitly labeled derived facts, the current
generation, and the current host-generated action IDs. It must not receive raw host objects,
executables, PCK/DLL bytes, saves, credentials, private prompts, screen coordinates, input events,
future RNG state, or unrevealed random outcomes. Model responses are parsed into a small decision
enum; verbatim output is not a trajectory artifact.

Live Exo connectivity, the selected revision, licensed STS2 build, and gameplay compatibility are
`unverified` until a separately recorded runtime handoff supplies exact build/configuration
lineage.
