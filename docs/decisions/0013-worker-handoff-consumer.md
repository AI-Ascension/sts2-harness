# ADR 0013: Harness worker handoff consumer

## Status

Implementation candidate under the watchdog assignment; independent review and
authenticated executable integration remain required. This is not a released endpoint.

## Ownership and contract

The watchdog owns deployment scheduling and supplies a retained job/attempt tuple.
The harness owns execution, provider accounting, checkpointing and terminal records.
Neither owner writes the other's database. A handoff identifier is distinct from
run, episode, trajectory and transport-request identities. Decoding a request does
not grant authorization, admit an episode or prove a terminal result.

The consumer pins the owner's neutral
[schema and manifest bundle](../../protocol-artifact/watchdog-worker-v1/README.md).
No sibling implementation crate or source is imported. The five closed request
commands are probe, dispatch, lookup, acknowledge and set-control-mode. Only an
empty parameter object for the approved Runtime-v3 episode operation is admitted
by this version; it is not an arbitrary process launch or gameplay interface.

## Validation and security

Before semantic use, the decoder bounds the frame to 65536 bytes and JSON nesting
to 16 child levels. It rejects duplicate decoded keys, trailing JSON, signed,
fractional and exponent number tokens and integers above 9007199254740991.
Field validation then enforces closed command-specific shapes, UUIDv4 canonical
spelling, pairwise handoff/run/episode/trajectory separation, byte limits, fixed
payload digest, exact schema digest and a 1..5000 ms requested timeout.

Errors are redacted and do not echo incoming content. The immutable request object
does not implement debug formatting. Transport authentication, capability checks,
actual deadline enforcement, current boot matching, persisted control sequencing
and durable admission must occur at the endpoint; these are not decoder guarantees.

## Evidence and remaining integration

The consumer tests execute all five positive request fixtures, all eight negative
fixtures, malformed numeric spellings, missing fields, duplicate escaped keys,
namespace collisions, byte limits and exact frame bounds. Fixture comparison
confirms all twenty imported JSON files match the owner snapshot byte-for-byte.

Response construction now binds the original request correlation, complete tuple
and current worker boot. Typed terminal response variants require a matching
closed receipt; nonterminal variants cannot carry one. Probe always reports
non-admitting readiness. A mismatched response command or worker boot is rejected.

Terminal acknowledgment uses lowercase SHA-256 of the complete WHJ-T1 terminal:
tuple keys in manifest order, followed by status, checkpoint sequence, terminal
reference and result digest. Compact UTF-8 JSON uses canonical unsigned integers,
escapes quote/backslash, retains other permitted Unicode bytes without normalization
and has no trailing newline. It excludes transport request/boot identifiers.
The hash is not the harness's private result digest. Both consumers test the same
frozen dispatch terminal against its independently calculated golden hash.

Inventory/manifest/schema tests additionally pin all twenty copied JSON files,
validate all positive fixtures and check the manifest's runtime limits. The
response suite tests all status shapes, terminal field corruption and boot/tuple
substitution. The durable worker store and actual server-to-runtime path remain
unfinished. No service, provider or game is launched by these tests. Conformance
is not end-to-end worker compatibility, release completion or live recovery.

## Transport framing adapter

The separate safe `worker_frame_io` module owns bounded asynchronous length-prefix
I/O, not the decoder or authentication policy. Its process-local connection budget
is created before connect/accept and peer checks, then moved into framing without
resetting elapsed time. A request timeout can only shorten it. The native endpoint
must apply the same instant to its earlier authentication and credential phases.

Each read validates the four-byte big-endian prefix against its phase-specific
bound before allocating a body; neither reads nor writes exceed 65536 bytes.
Zero-length, oversized, truncated, expired or cancelled exchanges poison the
connection so partial messages cannot be resumed under a new prefix. There are no
spawned I/O tasks, detached readers or per-frame timer resets. Errors expose no
payload, credential, OS message, path or peer identity.

Duplex-stream tests exercise the exact maximum size through partial I/O, prefix
rejection before body consumption, elapsed pre-framing budgets, failed timeout
extension, partial-read/write cancellation, EOF, stalled readers/writers and typed
redacted errors. These are adapter tests only: OS peer verification, credential
admission, native Windows cancellation, server wiring and runtime execution remain
separate required gates.
