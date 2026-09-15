# ADR 0022: bounded Exo lookup tool exchange

Status: accepted for the additive harness-owned source and synthetic process lane.
Owner: sts2-harness. Related issue: [#127](https://github.com/AI-Ascension/sts2-harness/issues/127).

`ExoLookupProcess` implements `LookupAgentPort` through private newline JSON protocol
`sts2.exo-lookup-wire-v1`. The existing bridge exposes `--lookup-describe`, `--lookup-run`
and `--lookup-synthetic` with distinct configuration `sts2.exo-lookup-config-v1`.
Its closed configuration pins the owned lookup extension, executor, Node executable and
unchanged Exo source revision `b06869ab789dee3f80ca474b5fa89dbe47ccb859`. The isolated
executor owns all upstream implementation dependencies. The terminal-only profile is unchanged.

The first frame carries a validated ordinary Exo decision request at sequence zero.
The executor emits `query` or `read_retained` frames at sequences 1 through 32;
the host replies with `feedback` at the same sequence. A `decision` carries only
`action_id` at the next sequence, at most 33. Each frame separately binds request and
turn identity. Duplicate JSON keys, unknown fields, wrong sequence and foreign identity
fail closed. Frames are at most 196,608 bytes, query arguments 16,384 bytes and feedback
7,000 bytes. ADR 0021's stricter applicable policy and producer limits still apply.
The generic harness loop admits at most 32 combined read/decision turns.

The extension registers exactly `sts2_lookup_query` and `sts2_lookup_read` through the
native TypeScript registry and injected `ToolRuntime`. It grants one model write initially
and rearms only after completed host feedback, with at most 33 writes and 32 tools.
Agent tool creation, external tool modules, arbitrary tools and namespaces are denied.
A fatal guard violation survives upstream retries. Native event evidence must match the
completed turn/session, contain one guard receipt, paired owned tool events, and one final
closed action object. The host independently checks unchanged legal IDs.

Provider arguments cannot supply owner scope, profile, snapshot, transport correlation
or live instance reference. The host injects those facts from the pinned `LookupBinding`
and rejects binding or legal-set changes between rounds. Game information is explicitly
untrusted data. Compact JSON text preserves feedback through the upstream 8,000-character
result wrapper. Oversized views become retained references; reads return at most 2,048
raw bytes encoded as hex with exact offsets and total size. Paging, archive identity and
no-fallback replay remain owned by `LookupSession`.

The process adapter owns a joined supervisor, bounded channels, absolute deadline and
process-group cleanup. Bridge stdin EOF cancels its separately owned executor while output
is pending. Early feedback, missing newline, crash, stalled output and forged decisions
are rejected. The process-owned stdin worker has bounded runtime shutdown. Credentials
remain a private stdin handoff, never tool input, model instructions, argv or public receipts.

The deterministic subprocess test drives the production agent loop through static lookup,
live detail and a legal decision using the admitted MCP mapping. A separate pinned Exo
oracle confirms two native tool round trips and three synthetic loopback model requests,
complete feedback in the next model input, and no private host IDs in that input.
The lane uses Node 22.14.0; upstream's declared 22.15.0 remains unverified.

Automatic main episode binding is not established by these tests. In consumed MCP
composition commit `03be7729635376325f2f6ee7c47c35896a742e0f`,
`mapping_game_information_request.rs` and `mapping_game_information_request_refs.rs`
require initial content and live snapshot identities. Its closed capability payload in
`mapping_game_information_shapes.rs` advertises limits and snapshot policy without
discovering those identities. The consumed runtime-v3 gameplay schema, SHA-256
`8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63`, supplies state ID and
generation without content-manifest or lookup snapshot identity. These namespaces must
not be equated.

Protocol PR 50, merge `c4f2258be73912f7e4b9b44d3ac0e3003fed3ebd`, adds a proposed v2
rest-option context candidate under ADR 0039, explicitly without admitted consumers or
runtime compatibility. MCP main `2567cd336` has no v2 adoption; the v1 request mapper
still matches the consumed revision. This candidate is not a qualified general identity
discovery path. Main episode integration needs an accepted producer/protocol/MCP source
of content manifest, locale and instance/run/epoch/snapshot identity, then harness wiring.
Native provider/game execution and unallocated content routing remain unverified/unsupported
respectively. Issue 127 remains open.
