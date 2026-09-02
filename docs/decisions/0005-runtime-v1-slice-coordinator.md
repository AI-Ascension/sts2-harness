# ADR 0005: `runtime-v1` slice coordinator

- Status: Accepted for component integration; host/runtime compatibility remains unverified
- Date: 2026-09-02

## Context

The harness has deterministic coordinator and artifact seams but no real MCP/gateway process path.
The next sprint needs an executable coordinator that can prove the cross-process sequence without
adding game access, provider access, or a second boundary authority.

## Decision

Add `sts2-harness-runtime` as a standalone Rust binary. It allocates one configured gateway lease,
spawns the configured MCP process with bounded stdin/stdout JSON-RPC, verifies the `runtime-v1` MCP
catalog, reads state at generation N, submits `show_runtime_probe` at N, repeats the action at N to
require the stable stale-generation rejection, reads fresh state at N+1, emits a sanitized trace, and
releases the lease. Instance, gateway session, MCP session, lease, epoch, request, and action
identities remain distinct.

The harness never contacts the game or mod directly and does not own gateway leases, host state,
action legality, effect settlement, provider execution, or profile cleanup. Tokens enter only through
operator environment configuration and are not printed or persisted.

## Consequences and evidence

The coordinator can exercise real MCP/gateway TCP behavior against a disposable synthetic downstream.
That component trace is `confirmed` when its acceptance checks and release cleanup pass. It does not
establish a managed host callback, Godot main-thread execution, STS2 effect, gameplay mutation,
provider run, or disposable game-profile claim; those remain `unverified`.
