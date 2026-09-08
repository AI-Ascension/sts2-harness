# Windows worker transport correction evidence

## Scope

Implementation candidate based on `4a54606`, with the accompanying local source
changes. Native synthetic Windows tests only: no installed service, gameplay,
provider, reboot, release activation, or watchdog/harness executable integration.
ADR 0014's complete integration gate remains open.

## Findings and corrections

- Windows path components normalize interior dot segments. Validate the original
  spelling before component traversal, including slash and mixed separators.
- Credential ACL inspection requires `READ_CONTROL` on the held file handle.
- Waiting on the retained peer process requires `PROCESS_SYNCHRONIZE`, in
  addition to limited process-query access.
- Local deadline cancellation must retain the deadline error after confirmed
  aborted completion; completion is still awaited before releasing buffers.
- Immediate server disconnect can discard an unread response. After writing,
  retain the connection while awaiting client EOF under the same absolute
  deadline. Extra bytes reject as framing; EOF is cleanup, not application
  settlement or a new acknowledgment contract.
- A slow-auth fixture must retain its partial prelude, not immediately exit.
- Hardlink creation against a read-only credential ACL fails with Windows error
  5 before reaching the oracle. Create a separate writable synthetic source and
  test the held-file hardlink gate directly, independently of ACL rejection.
- Symlink creation is an explicit privilege-dependent ignored test, no longer a
  silently conditional assertion inside a passing test.

## Observed results

Baseline native run: 9 passed / 7 failed. Subsequent fixes exposed the response
disconnect race and fixture failures rather than treating their failures as
security rejection evidence.

Build command:

```text
cargo test --locked -p worker-ipc-windows --target x86_64-pc-windows-gnu --lib --no-run
```

The resulting executable was copied to a local Windows filesystem and executed
with `--test-threads=1 --nocapture`: **17 passed, 0 failed, 1 ignored**, 8.80 seconds,
exit 0. SHA-256:

```text
b3f194866557ee6d07b4ea9a63ffa00cbfc83bb9ca9d001eba690dd21ac71bd1
```

The count includes pure policy and injected completion tests and the no-op parent
invocation of the peer fixture. It is not a count of 17 end-to-end exchanges.
Native tests exercise a valid peer, malformed authentication, a stalled prelude,
PID/creation/SID rejection, active-reader shutdown, endpoint ownership, hardlinks,
and a restricted pipe ACL. The explicit symlink test was not executed here.

## Remaining gates

Complete peer death/image mismatch, PID reuse, ancestor/path replacement,
stolen-name rearm, slow-writer, per-phase deadlines, and handle-cleanup coverage.
Independent review and the complete ADR 0014 fault matrix remain required.
These results do not resolve the watchdog client's separate bounded-I/O gate or
prove a real cross-repository handshake, service recovery, or loaded-image integrity.

## Response lifetime regression

The subsequent native regression delays response reads by 150 ms and separately
holds a connection open for 800 ms after receiving the response. The server's
original 500-ms deadline must expire while the latter child is still running;
both children must report receiving the exact response. This is controlled
finite-peer timing evidence, not a scheduler-independent timing guarantee.

Focused test: 1 passed in 2.05 seconds. Full native rerun: **18 passed, 0 failed,
1 ignored**, 10.55 seconds. Executable SHA-256:

```text
f397c70a8166d1f42364823127fddaf6e6afb72c1ebf43689c8eaa952b8abba8
```

Windows package Clippy with all targets/features and warnings denied passed.
Repository strict policy passed with 478 sized files and no warnings/errors.
The Linux invocation of this Windows-only package ran zero tests; it supplies no
native transport evidence. Full Linux workspace validation is recorded separately.

Full Linux workspace `cargo test --workspace --all-targets --all-features --locked`
completed with exit 0 on this correction snapshot. Formatting and `git diff --check`
also passed. This validates this isolated transport worktree's existing harness
baseline, not the newer, separately maintained executable-auth-bridge candidate.

Full Linux workspace Clippy (`--workspace --all-targets --all-features --locked
-- -D warnings`) passed. A subsequent native test,
`native_same_bytes_different_image_file_is_rejected`, passed in 0.78 seconds:
the listener successfully prepares an approved same-digest copy, then rejects
the actual peer's different executable file identity despite correct PID, SID,
and creation identity. This covers file-identity substitution, not loaded-memory
integrity or process hollowing. The full native-suite count above predates this
additional test.

Final native suite including the image-identity regression: **19 passed, 0 failed,
1 ignored**, 10.96 seconds, exit 0. Executable SHA-256:

```text
264942ad51953e7284719e3e2872084677cf09ddf8e713e9522e36fecee31259
```

Independent correction review reproduced the same executable hash and native
result (19 passed, 1 explicitly ignored, 11.01 seconds), and passed formatting,
Windows package Clippy with warnings denied, and diff whitespace checks. It found
no blocker to committing this isolated correction. This is not ADR 0014 integration
approval. Client EOF cannot distinguish consumed bytes from an early close and
must never be used as application-delivery or settlement evidence.

After the correction commit, the response-lifetime test gained an adversarial
client that sends an extra byte after reading the response. The server must
return `Framing`, and both a second request read and a second response write must
return `Closed`. The finite native test passed all three response-lifetime cases
in 2.63 seconds. This is additional single-connection framing evidence, not
completion of the remaining ADR 0014 fault matrix.

The native `native_authenticated_peer_death_closes_partial_request` regression
authenticates an owned finite child, terminates and reaps only that child, then
asserts the incomplete request returns `Closed`. Repeated request reads and a
response write also return `Closed`. The focused test passed in 0.54 seconds;
Windows package Clippy with warnings denied passed afterward. This covers death
after authentication during an incomplete request, not PID reuse, inherited pipe
handles, or death at every exchange phase.

After moving response tests into their own safe test module, the full native suite
passed 20 tests with 1 explicit ignore in 11.80 seconds. A subsequent
`native_unread_maximum_response_is_deadline_bounded` test passed in 1.34 seconds:
the peer never reads a maximum-size response and remains alive when the original
500-ms exchange deadline expires. This verifies the end-to-end response deadline;
it does not independently identify whether native writing or EOF waiting consumed
that deadline. The peer is finite and reaped by the test.

The full native suite including the unread-response case passed **21 tests,
0 failed, 1 ignored**, in 13.79 seconds. Executable SHA-256:

```text
237de396663a6951b76446bf712fc06084ea35ba62eeaccf5456fb07aa6458ec
```

A subsequent `native_held_image_blocks_file_write_and_ancestor_rename` regression
passed in 0.71 seconds. It uses only a disposable copy of the synthetic executable.
Writing that file and renaming its immediate ancestor fail while the image guard
is held, and both succeed after release. The positive controls distinguish held
sharing protection from a permanently restricted fixture. This is not a concurrent
reparse replacement campaign or protection against loaded-process memory changes.

Full native rerun including image-lock coverage: **22 passed, 0 failed, 1 ignored**,
15.08 seconds, exit 0. Executable SHA-256:

```text
317728a39737fc382b5789a11e678c2b17ae3a45afb8206e4b6ade46ab92a7a9
```

The subsequent `native_accept_timeout_rearms_exclusive_endpoint` test passed in
1.51 seconds. Two no-client accepts expire under their 50-ms budgets. After each,
a competing bind of the same endpoint fails; after shutdown and owner drop,
replacement binding succeeds. This covers observed post-rearm ownership and
accept timeout, not an adversarial race inside the disconnect-to-rearm interval.

Final test-batch native run: **23 passed, 0 failed, 1 ignored**, 18.63 seconds,
exit 0. Executable SHA-256:

```text
4fc90345821d7cde863ed0e5a9690a3161eee6f2fcf3da8a0ac6ad30ed7d795a
```

Final formatting, strict policy (480 sized files, no warnings/errors), and diff
whitespace checks passed. The prior Windows package Clippy run includes the rearm
case and passed with warnings denied.

Independent review reproduced the final executable hash and 23-pass/1-ignore
native result, and passed formatting, diff checks, and Windows package Clippy.
No correction-level blocker was found for this isolated test batch. The broader
integration gates and evidence limitations above remain unchanged.
