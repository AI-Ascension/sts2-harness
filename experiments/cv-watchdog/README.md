# CV watchdog (independent disagreement signal)

This experiment consumes an operator-approved screenshot or video fixture and emits a bounded
disagreement record. It is an observation-only watchdog: it never supplies a hidden state, legal
action, target, RNG value, or alternate authority to the policy loop.

The watchdog must report `unverified` when the fixture, model, or calibration lineage is absent.
It must not click, inject input, inspect process memory, read saves, or retain credentials. Production
episodes continue only on the semantic host/MCP observation path.

Run the planned probe only in a disposable authorized environment after recording the exact fixture
digest, detector revision, threshold, platform, and retention decision. The current workspace has no
CV provider or valued game fixture, so live watchdog evidence is unverified.
