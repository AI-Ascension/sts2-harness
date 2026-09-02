# Security Policy

## Supported state

There is no released harness artifact yet. Security fixes apply to the current development line;
future releases will state their support window.

## Reporting a vulnerability

When private reporting is available for the hosted repository, use its private vulnerability
channel. If it is unavailable, open a minimal public issue asking for a private contact path without
including exploit details, credentials, prompts, model output, saves, personal paths, multiplayer
identifiers, or other sensitive material.

Include the affected commit or version, boundary, impact, reproducible conditions, and the smallest
safe proof. Maintainers should acknowledge receipt privately, coordinate a fix and disclosure window,
and credit the reporter when requested and appropriate.

## Harness-specific risk boundary

The harness coordinates actions that can ultimately request game mutations through the MCP and
gateway boundaries. Treat credential propagation, confused-deputy behavior, cross-instance leakage,
stale leases, replayed actions, provider egress, prompt/model-data retention, artifact exposure,
unbounded inputs, path disclosure, and cancellation ambiguity as security-relevant.

The harness must never bypass gateway or MCP authority, persist provider credentials in trajectories,
export private data by default, or test against a valued profile, save, public service, or another
person's instance without explicit authorization. Security tests fail closed when required policy,
fixtures, or authorization is unavailable.
