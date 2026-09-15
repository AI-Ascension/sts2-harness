# ADR 0029: Owner lease tolerates a transiently held lock description

## Status

Accepted for the scoped fix of Harness #188. The lease primitive, its exclusivity and its
crash-release behaviour are unchanged.

## Context

`crates/harness/src/exo_lifecycle/tests.rs` intermittently failed under the parallel lib test
runner, panicking with `restart: Busy` where the test re-opens a journal it has just released. The
issue proposed two hypotheses: per-test lease/journal state, or the global injection seam in
`provider_session/owner_journal/io.rs`.

Both were investigated and refuted:

- the commit-failure injection seam is already `thread_local!`, so it cannot leak between parallel
  tests;
- the fixture root was time-based, but a collision would fail in `create_dir`, not in the lease.

Diagnostics at the failing point showed no live lease in the process, no other descriptor on
`owner.lock`, and a retry milliseconds later succeeding. A standalone reproducer (300,000
iterations of `create_new + try_lock + drop`, then `open + try_lock`) reported zero spurious
`EWOULDBLOCK` with no concurrent child spawns and 1,031 with a background `Command::spawn`, every
one of which succeeded on a 5 ms retry.

## Decision

`Lease::acquire` retries the non-blocking attempt up to `LEASE_ATTEMPTS` (32) times with
`LEASE_RETRY` (5 ms) between attempts, mirroring `map/bundle_store_io.rs`, which already acquires
the map publication lock this way. Exhausting the attempts still returns `LifecycleError::Busy`, and
a lock error is still mapped to `LifecycleError::Unsupported`.

An `flock` belongs to the open file description, not to the process, and a spawn gives the child a
copy of the parent descriptor table. A descriptor this process has already closed therefore keeps
the lock alive until the child reaches `execve` and drops its `CLOEXEC` copy. While any test in the
same binary spawned a child, an immediate non-blocking attempt observed `EWOULDBLOCK` with no owner
holding the lease.

The fixture root is additionally derived from the process id, the clock and a process-wide counter
rather than the clock alone, because clock granularity is not an isolation primitive.

## Compatibility

The lock file, the locking primitive, the `Lease` exclusivity invariant and its release on process
death are unchanged. The only observable difference is that a genuinely busy lease is now reported
after up to 160 ms instead of immediately, which is the interval the existing map publication lock
already uses.

## Consequences and limits

- Retrying cannot admit a second owner: a lock held by a live `Lease` or by another process is not
  released by waiting, so acquisition still ends in `Busy`.
- The wait is bounded and fail-closed, so contention is reported rather than hidden.
- `crates/harness/tests/exo_lifecycle_lease_contention.rs` covers a transient holder generally; the
  spawn window itself is reproduced by the standalone reproducer recorded on #188 rather than by a
  timing-dependent test.
