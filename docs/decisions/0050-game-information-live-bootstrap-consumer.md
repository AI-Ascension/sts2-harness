# ADR 0050: live observation bootstrap consumer

The Harness consumes `game-information-live-observation-bootstrap-v1` after lookup-binding
discovery and observation. A provider turn supplies one concrete definition reference and may
supply an exact occurrence. The runtime supplies the authenticated instance, run, authority,
manifest and locale scope from the current owner before sending the fixed gateway POST route.

Responses are bounded and checked for protocol/schema pins, scope, owner provenance, parent
observation, visible entities and per-entity snapshot identity. An omitted occurrence is accepted
only when exactly one visible entity matches; ambiguous or missing entities remain unavailable.
The selected snapshot is retained until the next lookup-binding observation or owner revocation.
The native occurrence epoch is an entity identity and is never used as the Harness authority epoch.

Bootstrap records are retained beside query records. Replay validates the typed transcript and
selected snapshot without MCP or gateway calls. The optional provider bridge uses the explicit
`sts2.exo-lookup-wire-v2-bootstrap` frame pin; the closed terminal-only v1 wire remains unchanged.
