# Harness provider-session contracts

These closed contracts are owned by `sts2-harness`. `policy.schema.json` describes the portable
provider-session policy ceiling (1,024 turns and 604,800 seconds of history); a selected native
profile may expose lower executable limits. `capabilities.schema.json` publishes every common
session/transport limit that consumers may rely on, plus the policy/model/adapter revision binding
and descriptor digest.

`NativeCapabilities::effective_limit_record()` publishes the same
`ascension.harness.effective-limits.v1` classification record described in
`docs/effective-context-limits.md`, including the tool that exposes it and the consumer pin matrix.

Portable validity and profile admission are separate checks. A policy within the schema ceiling
but above `effective_limits` is retained for inspection and rejected before a candidate/session
is admitted. Missing, malformed, stale, or mismatched capability metadata is never interpreted as
unlimited. The descriptor digest is an integrity marker, not producer or freshness authentication;
callers establish staleness relative to trusted configuration by pinning expected revisions through
`validate_against_trusted`.
