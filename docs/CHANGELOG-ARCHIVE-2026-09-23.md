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
