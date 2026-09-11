# Decision: harness-owned prepared context dispatch

Status: accepted for the Phase 2 fixture branch

The harness keeps provider and game authority at its existing ports. The context-control module
adds an immutable prepared-input value and a small control authority beside those ports. Its legacy
profile returns the existing bytes unchanged. Its enabled profile serializes attributed notes and an
authorized objective into bounded model-visible context, binding the adapter revision and boundary
digests.

ExoSession::decide_prepared checks the reserved execution identity and adapter revision, then sends
the exact approved bytes through the existing transport/capture seam. The Ollama bridge delegates
its user content projection to the same library module, so enabled context reaches the actual HTTP
body while legacy requests keep the old body. Neither path can authorize a game action; host
legality and settlement remain outside this module.

ControlAuthority uses idempotent command receipts, a pause barrier, revision/plan fencing, an
explicit resume, stop dominance, and a bounded JSON journal for restart recovery. Live providers,
host processes, and deployment are outside the fixture evidence.
