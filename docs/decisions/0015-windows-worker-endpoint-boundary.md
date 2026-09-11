# ADR 0015: Windows Worker Endpoint Boundary

## Status

Accepted for the native worker IPC adapter. Native Windows execution, service
installation, reboot recovery, and live harness compatibility remain unverified.

## Context

The harness worker endpoint must authenticate the watchdog-owned peer before it
reads a credential or accepts a Runtime-v3 command. Linux can use Unix peer
credentials, pidfds, `/proc` image identity, and an owner-only socket. Windows
needs an equivalent named-pipe, process-token, image, session, and protected-file
contract. The harness must not gain game, host, loader, managed-assembly, or
gateway implementation authority while providing that platform adapter.

## Decision

`crates/windows-worker-boundary` is the separately owned Windows FFI boundary.
It is a target-only dependency of `sts2-harness`; it has no cross-repository or
game implementation dependency. The parent harness keeps bootstrap policy,
Runtime-v3 admission, durable records, cancellation, and child lifecycle
coordination free of unsafe code.

The boundary exposes only typed operations:

- an exact nonce-bound `\\.\pipe\ascension-worker-*` endpoint with a protected
  owner SID DACL, local-client rejection, first-instance creation, bounded
  instances, and polling I/O;
- peer PID/session lookup, process creation-time/path/token-SID checks, a held
  non-reparse executable image, bounded SHA-256 admission, and identity checks
  before and after every transport operation;
- an owner-only credential reader requiring a protected DACL with exactly one
  non-inherited allow ACE for the current SID, bounded reads, and zeroizing
  returned bytes; and
- bounded inherited-stdin bootstrap framing with exact magic, length, EOF, and
  deadline checks.

The FFI invariants are narrow: every kernel or security-descriptor handle has
one RAII owner; raw handles and Win32 structs do not cross the crate boundary;
mutable handles remain on their owning thread; retained executable and peer
image handles are not writable or deletable through later opens; no credential,
path, SID, or raw process data is emitted in `Debug`; and all frame, path, SID,
ACE, image, and polling bounds are checked before allocation or external use.
The adapter does not contact a game process or host object and does not create
an alternate lease, MCP, provider, or mutation authority.

## Evidence and follow-up

The boundary and the Windows harness path pass pinned Rust formatting, strict
Clippy, and target `x86_64-pc-windows-gnu` Rust checks in the available
environment. A native Windows runner, linker, service/SCM host, and live peer
are required before native endpoint, launch, or reboot evidence can change from
`unverified`. The adapter must not be described as native or live evidence until
that gate is run and its exact build, host, peer, and artifact identities are
recorded.

Revisit this decision if the worker must launch from a kernel image handle,
needs Job Object or service ownership, exposes a new raw Win32 operation, or
requires host/game access. Such changes need a new boundary review and an
updated threat model.
