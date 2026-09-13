# Harness provider-session contracts

These closed contracts are owned by `sts2-harness`. `policy.schema.json` describes the portable
provider-session policy ceiling (1,024 turns and 604,800 seconds of history); a selected native
profile may expose lower executable limits. `capabilities.schema.json` publishes every common
session/transport limit that consumers may rely on, plus the policy/model/adapter revision binding
and descriptor digest.

Portable validity and profile admission are separate checks. A policy within the schema ceiling
but above `effective_limits` is retained for inspection and rejected before a candidate/session
is admitted. Missing, malformed, stale, or mismatched capability metadata is never interpreted as
unlimited.
