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

The broker metadata journal is versioned as ascension.provider-session.broker-snapshot.v1. Its
strict, duplicate-key-checked JSON is bounded, retains operation idempotency, binding/event
epochs, maintenance records and bounded history projections (including redaction flags), and
restores only with a fresh owner token. Prepared turn bytes and in-flight turns are deliberately
not serialized; recovery therefore remains held and requires explicit reconciliation rather than
automatic replay or scheduler resume. This is a fixture-side metadata restore, not evidence of
native encrypted-store durability.

The broker journal can be kept explicitly volatile or written through the
`ProviderSessionMetadataStore` encrypted-persistent adapter. That adapter authenticates a bounded
XChaCha20-Poly1305 metadata envelope, validates a private owner-checked path, atomically replaces
the file, and requires exact scope/policy/profile matching on restore. It contains no prepared turn
bytes and does not encrypt, redirect or attest to native Codex state, rollout files, WAL/log files
or temporary files.

The owned stdio transport clears the parent environment and binds conventional home/config/cache,
Codex and temporary roots to the approved private state directory. The inherited-environment list
cannot override those names; native instruction precedence, OS containment and quota observation
still require an independent native capability review. Startup also scans the complete private tree
against the 256 MiB bound and fails closed on symlinks, special files, unsafe child roots or
over-limit bytes, with explicit entry and depth bounds; active-worker growth remains a separate observation requirement. Configuration
allows only the approved `OPENAI_API_KEY` secret name through that boundary and rejects unrelated
credential, endpoint and path variables.

The native Codex App Server profile, provider authentication, encrypted OS state, native binary
compatibility and remote erasure require a later capability review; the fixture evidence does not
claim any of them.
