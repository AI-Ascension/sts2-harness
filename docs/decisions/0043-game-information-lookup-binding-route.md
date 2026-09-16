# ADR 0043: game-information lookup-binding route

## Status

Accepted for the bounded `sts2-harness#127` discovery and observation slice.

## Context

The pinned `sts2-protocol/game-information-lookup-binding-v1` artifact defines the binding
identity and discovery/re-observation semantics, but deliberately does not choose a transport.
The Runtime-v3 episode adapter needs one owner-backed caller path before it can admit the existing
read-only game-information MCP tools. The harness cannot contact the game mod or host directly, and
the MCP server cannot become a second lifecycle or lease authority.

## Decision

The gateway serves the fixed owner route:

```text
POST /v1/instances/{instance_id}/game-information/lookup-binding
```

The harness sends the closed request:

```json
{
  "operation": "discovery | observe",
  "project_id": "harness-owned",
  "run_id": "harness-owned",
  "episode_id": "harness-owned",
  "agent_id": "harness-owned",
  "authority_epoch": 1
}
```

`operation` is closed to `discovery` and `observe`. The request has no locale, game profile,
manifest, snapshot, binding identifier, or caller-selected lease. The gateway validates the path
instance, authenticated session, current lease and fence, and supplies locale. It routes the request
to the game-mod adapter, which supplies `game_profile`, `content_manifest_id`, authoritative
snapshot identity, and `state_generation`. The MCP server exposes only the read-only
`sts2.game_information_binding` tool over this route; it does not own a second route or discovery
authority.

Before starting the MCP child, the harness serializes the existing closed discovery request as
compact JSON in `STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON`, bounded to 2048 bytes. The request
contains only the validated lookup scope, `operation: "discovery"`, the fixed discovery correlation
identifier, and the selected memory-policy owner's `binding.fence.owner_epoch`. It is derived from
one authenticated active-policy snapshot and that same `ActivePolicyBinding` remains the expected
binding through Gateway discovery, observation, and lookup-session construction. The value carries
no owner credential, store key, grant, endpoint, or producer offer; the MCP child environment is
cleared and receives this one value explicitly. The ordinary observe request is never accepted as
startup input. Owner-backed startup does not parse or use `STS2_AUTHORITY_EPOCH`.

The LBR `authority_epoch` represents the Harness-owned lookup-policy authority epoch from that
selected binding. It is distinct from the policy store's durable `store_epoch`: reopening the store
advances `store_epoch` and fences the old selected snapshot until explicit revalidation and adoption,
even when the scoped LBR authority epoch remains unchanged. Each Gateway discovery or observation
call is checked against the retained `ActivePolicyBinding` before and after network I/O, without
holding the owner lease during the call.

The MCP startup bootstrap and Harness retained-session setup each perform a read-only discovery
exchange. The bootstrap validates the producer-backed catalog before MCP advertises it; Harness
then performs its own discovery and observation and admits queries only against that validated
binding and current selected-policy fence. These are separate reads, and the fixed discovery
correlation value matches each request to its response; it is not an idempotency key.

The harness consumes the exact artifact profile
`game-information-lookup-binding-v1` and schema digest
`f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58`.
The consumer copy is pinned to protocol `843fddbd3b5875d01d7f99d3433f872b0a4d7681`; the
source-level producer and adapter assumptions are gateway `b6b94bf`, MCP `2567cd3`, and game-mod
`a8ccb8b`. These are component source pins, not deployment or host-compatibility evidence.
It recomputes the binding identifier, fences scope, instance, and epoch, requires discovery before
observation, and retains no producer authority. A stale response is discarded and triggers one
same-binding observe call, which must yield a new observation identifier. An exhausted response
fails closed. A missing gateway route or native adapter is reported as
`game_information_binding_unavailable`; it never falls back to synthetic game data.

Runtime-v3 performs discovery plus the initial observation only when
`STS2_ENABLE_GAME_INFORMATION_LOOKUP_BINDING=true`. This keeps the existing runtime profiles
compatible while giving the deployed entry point an explicit, operator-selected caller path.

## Bounds and conformance

Gateway request bodies remain within the Runtime-v3 HTTP client's 16 KiB bound. Every identity
string is validated by the runtime configuration's 128-byte safe-identity bound; `authority_epoch`
is a positive safe unsigned integer. The runtime's focused transport test proves the fixed POST
path, closed body, and selected MCP-session header. The entry test also proves that the bounded MCP
startup request uses the adopted owner's scope/epoch despite a conflicting inherited epoch/value,
and that the same selected binding fences subsequent calls. The consumer tests use the copied LBR v1
discovery, reobserve-required, reobserved, and exhausted goldens, including a forged-binding
rejection.

This is synthetic component evidence for harness routing and consumer behavior. It does not prove
gateway route registration, MCP tool registration, game-mod extraction, native host compatibility,
or an agent run until those owners publish their corresponding pins and conformance results.

## Consequences

The existing game-information query profile remains separate. This decision does not define a
complete content-manifest transport: package/version/order/inventory/unhandled/override payloads
are absent from LBR v1 and require their own versioned protocol contract.
