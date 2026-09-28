# Changelog archive: 2026-09-28

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- **Size the jev process-teardown pipe-cleanup bound above host-load jitter.** The paired runner gave
  a killed process group 250 ms to close an inherited pipe and reported `child_closed: false` past
  that, but a clean host's kill-to-close tail already reaches 250-306 ms under load, so the flag read
  "closure unconfirmed" for a process that closed a millisecond later and the offline runner-process
  contract test flaked. The grace is now a documented 1000 ms contract value, and a new escaped-
  session control proves the flag stays false when a bound is genuinely spent, so the assertion was
- Add Linux [Jev streaming mode](../experiments/jev-plays-sts2/STREAMING.md): retain the game with manual resume; preserve timed benchmarks. Automatic terminal progression remains unavailable.

- **Hold one live decision attempt so a lost reply cannot buy a second one.** A live `Decide` node
  took a fresh `ModelExecutionId` on every entry and kept no record of the attempt, so an
  `Unresolved` refusal left the run `NeedsOperator` with `pending_operation: null` and the next
  `Step` paid the provider again. The attempt is now installed and durably recorded before
  `decide_for`, released only by a refusal the provider owner reported before it could write, and
  re-used by a retry that reproduces the admitted request digest. Compatibility: additive; no wire or durable record changes. See [ADR 0059](../docs/decisions/0059-held-live-decision-attempt.md). Refs #108.

- **Route the executable REST selector composition into the runtime peer contract lane.** The
  `runtime_v4_rest_executable_composition` witnesses (issue #148) assert the authored
  observe → decide → execute-action → terminal graph settles each durable REST receipt before it
  advances, but no gate invoked them, so the regressions they pin were as invisible as if they had
  never been written. The lane now runs both against the pinned gateway and MCP peers, each with
  its own evidence directory so a REST run cannot overwrite the generic composition's
  `result.json`. The peer-binary environment those steps repeated moves to one job-level `env:`
  block, which keeps the workflow inside its nonblank-line budget after the addition, and the
  fail-closed lane check now covers both composition binaries. Refs #148.

- **Execute the idle-adoption policy-rebind regression in the runtime peer contract lane.** The
  `served_decision_survives_changed_policy_adopted_while_idle` witness was written for the permanent
  mid-run policy-adoption fence (issue #255) with the same `#[ignore]` operator marker as its
  siblings, but no lane step invoked it, so the regression was as invisible as if it had never been
  written. The served policy step now also runs it against the pinned gateway and MCP peers, and a
  fail-closed lane check rejects a declared operator-only composition test that no lane step executes
  or a lane `--exact` invocation that names no declared test. Refs #255.
- **Hold the `LivePeers` gateway port until the child is spawned.**
  `runtime_v3_game_information_entry_live_peers.rs` drew its gateway address with a
  `free_address()` that read `local_addr()` and dropped the listener inside the same expression, so
  the port was unowned from that instant until the pinned Gateway bound it. That is the `#673`
  defect, and `#693` fixed it in `tests/support/**` only. This is the third `src/bin` instance; the
  other two are `#681`, which does not name this file, so closing `#673` retired the only tracker
  for a defect that was still live.
  It is reachable end to end: `LivePeers::start` hands the address to a real spawned Gateway through
  `STS2_GATEWAY_ADDR` and then waits on `wait_until_listening`, whose success condition is
  `TcpStream::connect(address).is_ok()`. Anything that took the port in the window -- a parallel
  scenario, another test binary drawing `:0` -- left the child's `bind` to fail with `EADDRINUSE`
  while the readiness check reported success, because it connects to whatever is listening rather
  than to the child.
  The allocator is now a reservation, the shape `#693` landed: the listener is held in
  `ReservedAddress` and released as the last statement before `Command::spawn()`. `#681`'s framing
  proposes "identical in shape to `#673`'s: return the bound `TcpListener`", and that was measured
  and refuted on this kernel -- a second concurrent `LISTEN` on one `addr:port` is refused under
  every socket-option combination, so a parent-held listener blocks the intended child too.
  The mod listener ten lines above is untouched and stays correct: it is bound, kept, and accepted
  on by `spawn_mod_server`, so that address is owned continuously. No test here is known to fail
  this way and none is claimed to; the fix is on the verified code shape and the verified reachable
  call path. Refs #701, #681, #673.

- **Admit the host-offered `continue_run` action in the runtime-v3 path.** A host that offered
  `continue_run` beside `start_run` failed the whole observation: the production runtime-v3 parser
  and the fair-play sanitizer both refused any action kind outside their allowlists, and the parser's
  kind table fell through to `save_quit`. Both boundaries now admit the host-owned identity with an
  optional `run_id` discriminator (`{"kind":"continue_run"}` or
  `{"kind":"continue_run","run_id":"profile1"}`), preserve the host-generated `action_id`, and still
  refuse a save path, a null or non-identity `run_id`, an unknown kind and an extra field. The
  allowlist is now the single source of truth for a kind's field contract and its typed action, so an
  unknown kind is rejected before dispatch instead of being coerced into another action, and the
  continuation is bound to the offered generation so a stale catalog is refused before any effect.
  Refs #390.

- **Freeze the host-offered `continue_run` admission contract and prove its consumer-first
  boundary.** The two accepted shapes and the refusal list are now recorded beside the runtime-v3
  admission (`payload_contract`), the Exo projection (`schema.rs`) and `docs/ARCHITECTURE.md`,
  citing `sts2-harness#415` (`551ec19d`) and `sts2-game-mod#210` (`8a655143`); focused tests cover
  the valid offer and the malformed, unknown-field, stale, foreign-profile and unoffered refusals. Refs #390.

- **Execute the shipped host-lease campaign downstream in the runtime peer contract lane.** The
  `the_env_configured_campaign_downstream_answers_a_signed_install` witness was declared
  operator-only and no step invoked it, so nothing in CI proved that the *environment-configured*
  long-lived `synthetic_mod_server` process advertises `host_lease=enabled` and terminates a signed
  `lease_install_request` — the half of the sideband a long campaign depends on, while only the
  in-process terminal was covered. The lane now builds that operator target, points
  `STS2_SYNTHETIC_MOD_SERVER_BINARY` at it, and runs the witness, and the fail-closed lane check
  pairs each lane with the operator marker its own source declares. Refs #94.
- Verify **the System One bridge's refusals at its own process boundary**, and pin the tie it does not
  resolve. Three acceptance criteria were carried unverified because nothing exercised the seam they
  named. A shell transport now drives the real `sts2-jev-bridge`: a non-`200` exit, a raw HTTP status
  line read as a body, a malformed envelope, an out-of-catalog choice, a well-formed answer past the
  128 KiB bound, and a transport failure are each refused with the bridge's exit status and no
  decision, and every case proves the provider answer arrived first, so a case whose transport never
  ran cannot pass. The operator credential is asserted both positively — `TYPESAFE_API_KEY` reaches
  the transport by name — and negatively, in a captured request, a record, `--describe` and a
  refusal. A local provider declaration's digest pin is exercised at the runtime boundary with a
  control that pins the digest those bytes really have, so the refusal is the mismatch and not the
  presence of a declared bridge. And two equally likely options are shown to be resolved by nothing:
  the answer's own `choice` is returned, the gate is applied to the confidence the provider stated,
  and the tied identifier named in the bridge-authored rationale is the one the sorted probability map
  orders last, so the same answer produces the same rationale whatever order the provider wrote its
  keys in. Refs #284, #285, #288.

- **Record the two source-only Jev-runner decisions.** The wall-clock-sensitive global-time-budget
  test's fixture strategy is recorded in
  [ADR 0063](decisions/0063-jev-runner-first-arm-admission.md): the first scheduled arm is
  admitted structurally rather than by fixture timing (#388). The 1,000 ms teardown cleanup bound is
  accepted as a host-load-dependent contract in
  [ADR 0064](decisions/0064-jev-runner-teardown-cleanup-bound.md), with the strict closure
  assertion intact and the sampling limitations retained (#394). The Jev-evaluation Node suite was
  rerun without retry masking; first-attempt results are in
  [the no-retry matrix evidence](evidence/jev-evaluation-noretry-matrix-20260923.md).
  Compatibility: documentation only; no code, record shape, or runner contract change.
  Refs #388, #394.
