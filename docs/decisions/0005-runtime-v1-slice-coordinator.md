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

## Runtime adapter safety correction

The existing Runtime-v1 coordinator uses pinned Tokio 1.53.1 only at its subprocess boundary for
cancellable asynchronous pipes and a current-thread runtime driven by joined scoped supervisors.
This replaces blocking pipe I/O that could outlive an exchange deadline. The enabled features are
`rt`, `process`, `io-util`, `time`, and `macros`; no provider or game API is introduced. Request and
response frames remain bounded to64KiB; a five-second budget covers both concurrent writes and reads.
Direct-child kill/reap has a separate250ms grace period. Child cleanup errors are explicit, and
descendant-owned pipes cannot strand harness workers; arbitrary descendant termination and OS
sandboxing remain outside the adapter's guarantee.

The MCP child receives only explicit STS2 connection configuration and PATH/SystemRoot/TEMP/TMP;
its protocol profile is explicitly Runtime-v1 and stderr is suppressed. Gateway plaintext traffic
is numeric-loopback-only, with a whole-exchange deadline and unambiguous HTTP framing. Outer RPC
identity and strict projected tool shape are checked before emitting a sanitized trace. MCP owns
full downstream envelope/version/fence validation; that metadata is intentionally absent from its
Runtime-v1 projection. Only a validated typed action rejection may carry `isError: true` through to
the stale-generation oracle; arbitrary tool errors still fail closed. Action trace stdout alone
does not prove successful cleanup; exit success additionally requires MCP close and lease release.
