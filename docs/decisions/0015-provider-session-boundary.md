# Decision 0015: owned persistent-provider session boundary

Status: accepted for the Phase 4 fixture branch

`provider_session` is an additive harness-owned boundary beside the stateless provider ports. It
keeps project/run/episode/agent scope, owner/session/history/revocation epochs, operation intent,
dependency closure and late-result fencing in typed records. The console receives only binding,
history and operation projections; a native thread ID is never a bearer capability.

The default policy is disabled. The fixture profile uses a compiled fake peer over an owned stdio
transport with the version-pinned JSON-RPC 2.0 line envelope, rejects server-initiated privileged
requests, and exposes no shell, file, network, MCP, skill or game operation. Its reviewed native
method subset is `initialize`, `thread/start`, `thread/read`, `turn/start`, `turn/interrupt`,
`thread/fork`, and `thread/compact/start`; local retirement is deliberately not sent as an
unreviewed native method. A candidate/reconnect/fork/compaction operation remains held until
the existing Phase 2 approval and explicit resume path authorizes a decision turn. Unknown sends
are not retried, and retirement is irreversible even if a late native response arrives.

The native Codex App Server profile, provider authentication, encrypted OS state, native binary
compatibility and remote erasure require a later capability review; the fixture evidence does not
claim any of them.
