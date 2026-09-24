# Changelog archive: 2026-09-23

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- Record the **System One provider lane and its transport** in
  [ADR 0053](decisions/0053-system-one-provider-lane.md). A System One provider evaluates typed
  questions against one state and returns structured answers with probabilities and a calibrated
  confidence rather than text, so a host-generated action catalog becomes the option set of one typed
  question and the returned distribution can gate the decision. The ADR admits it as a local bridge
  kind on the legacy lane — digest pin, argument allowlist, and explicit combat gate intact, no
  live-episode promotion, no reviewed-envelope admission, whose route axes bind one provider and host
  by design — and decides that the bridge owns the request and answer contract while a pinned,
  operator-owned executable owns the HTTPS exchange, following the precedent `sts2-astra-bridge` set.
  A pure-Rust TLS client inside the bridge is recorded as the migration path with the reasons it is
  not the first step, and the rejected alternatives are stated rather than implied. Published limits,
  price, and documented model weaknesses are carried with `source-derived` labels and their sources;
  the claim that this lane plays the game is `unverified` and no run exists. Compatibility:
  documentation only; no code, dependency, or contract changes. Refs #286.

- Share one **bounded HTTP/1.1 response reader across provider bridges**. The strict reader that
  refuses oversized headers, a duplicate `Content-Length`, both framings at once, a non-`chunked`
  transfer coding, an oversized or short chunk, and any trailer after the terminal chunk moves from
  the Ollama bridge's private `runtime_support` include to the shared `bin/support` tree, where a
  second bridge reaches it the same way the Astra bridge reaches its accounting support. Refusals are
  now a typed `ProviderResponseError` carrying a stable code per cause rather than an opaque string,
  and the added negative tests pin the status, terminator, declared-length, absent-framing,
  malformed-header, and non-JSON refusals that were previously only implied. The loopback-only
  `ManagementClient` stays a separate boundary and is unchanged. Compatibility: no behaviour change;
  `sts2-ollama-bridge` accepts and refuses exactly what it did before. Refs #282.

- Compute **exactly derivable combat facts** from an admitted observation, so a provider is handed
  comparisons rather than operands. `context_control::DerivedExactFacts` states gross incoming
  damage (revealed intent damage times hits, only when every listed enemy carries an intent), a
  `fatal`/`heavy`/`survivable` label against current hit points, the hand cards current energy
  covers, the hand cards whose cost is not a fixed number, the single lowest-hit-point enemy, and
  the two counts a model would otherwise tally itself. It reads the admitted observation only, and
  a value it cannot derive exactly is omitted rather than estimated: an unrevealed intent removes
  the damage total and says so instead of counting as zero, and a tie names no weakest enemy. Two
  boundaries come from the declared model-view vocabulary rather than from the game — a card carries
  no attack value, so no lethal claim is derivable, and nothing carries block, so incoming damage is
  gross and named to say so. The projection is `derived_exact` under the fair-play taxonomy and is
  not host authority. Compatibility: additive; one new module and its re-exports, no change to an
  existing record, route, or digest. Refs #287.
