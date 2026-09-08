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

Response encoding, terminal digest acknowledgment, the durable worker store and
the actual server-to-runtime path are unfinished. No service, provider or game
is launched by the decoder tests. Request conformance is not end-to-end worker
compatibility, release completion or live recovery evidence.
