# Harness pin of `watchdog-host-lease-control-v1`

This directory is the harness-owned copy of the neutral
`watchdog-host-lease-control-v1` artifact. Every file except this note and
`SHA256SUMS` is byte-identical to the published artifact, and the artifact's own
`PROVENANCE.md` lists the same SHA-256 value for each of those files.

## Source

```text
repository  AI-Ascension/sts2-gateway
revision    77ffb01aef50e8e53b3bd2fb015074c649c3a383
path        contract-artifact/host-lease-control-v1/
license     MIT
```

## Why the harness pins it

The artifact's own manifest records `consumer_owners: ["gateway", "host-mod"]`.
The harness is a host-mod consumer in one specific place: `sts2-gateway` drives
every recovery mutation to the mod address over the fixed
`POST /api/v1/runtime/recovery` mux, so the long-lived synthetic downstream the
soak campaign runs has to terminate that hop as the signed host. Without a host
half, the durable recovery path has no peer and the repeated-episode profile is
unreachable.

`crates/harness/tests/support/host_lease_control_canonical.rs` re-derives the
HCJ-1 canonical form and the
`HMAC-SHA256(key, UTF8(domain) || 0x00 || HCJ1(frame without auth.proof))`
recipe from this pin rather than from any consumer's source, and
`crates/harness/tests/host_lease_control_conformance.rs` executes the pinned
vectors against it.

## What the pin does and does not establish

The vectors classify themselves as `test-vectors-not-live-evidence`. Reproducing
them shows that the harness host terminal agrees with the published profile on
canonicalization, proof domains, and proof bytes. It shows nothing about a
deployed host, a protected key, live gameplay, or an approved soak window, and
the deterministic `test_key_hex` in `proof-vectors.json` is public test data
that must never be a deployment credential.

The gateway also accepts the standard base64 spelling of the 32-byte host lease
key. The harness terminal accepts the 64-hexadecimal spelling only and refuses
the base64 spelling by name.
