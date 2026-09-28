# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md) and the dated archives beside it,
including [`docs/CHANGELOG-ARCHIVE-2026-09-28.md`](docs/CHANGELOG-ARCHIVE-2026-09-28.md).

## Unreleased

- **The review-of-record gate is no longer unmergeable after its own review lands.** The
  `concurrency` group added for #729 was keyed on the pull request number and head SHA but not on
  the event name, so a `pull_request` run and a `pull_request_review` run for the same head landed
  in the same group. Measured on #733 head `4b20402e`: push run `36447039873` was created at
  15:53:46Z and ran until 15:57:28Z, and review run `36447155603` — the **newer** of the two — was
  created at 15:54:40Z and cancelled at 15:54:56Z. `cancel-in-progress` gated on the event name did
  not prevent it, because that expression only decides whether the *arriving* run supersedes; it
  cannot stop an in-flight predecessor from cancelling the arrival. Since the review trigger is
  the only path that turns the check green, the head was left carrying a cancelled
  `review-of-record` beside a green run of the same context, and under ruleset 24104281 the
  required check resolved to the cancelled run, so the merge API refused. The group key now carries
  the event name, giving the two triggers separate groups: a review submission can no longer be
  cancelled by a push, while a push still supersedes its own earlier push run and the head SHA
  still prevents a push from cancelling a previous head's run. Refs #735.
- **A durable exchange that completed is no longer reported as a turn timeout.** The admitted
  transport re-measured the caller's ceiling against its own wall clock *after* the inner
  transport returned, so an exchange the inner effect had already completed inside its own
  deadline was reported as `Timeout` whenever the surrounding host was slow. That is the exact
  ambiguity the admission layer exists to remove: the inner transport already owns the deadline
  and can tell an overrunning peer from a slow scheduler, so the adapter now passes the ceiling
  down and surfaces the inner verdict instead of re-deciding it. The admission tests that assert
  a *successful* exchange were also handed the fixture's own effect budget, which made "the host
  was slow" indistinguishable from "admission refused"; they now use a ceiling above the effect's
  budget, and timeout behaviour is asserted deterministically against a peer that overruns.
  Refs #716.
- **The review-of-record gate's runs are now serialised per head.**
  `review-gate.yml` triggers on `pull_request` as well as `pull_request_review`, and without a
  `concurrency` group a slow push-triggered run overlaps the review-triggered run for the same head
  instead of yielding to it. The group is keyed on the head SHA as well as the pull request number,
  so a push never cancels work belonging to the previous head, and `cancel-in-progress` is gated on
  the event name so a review submission is never cancelled, since that trigger is what turns the
  check green. This is queue hygiene: it does **not** clear a completed red run, and it does not
  make a required check resolve against the newest run. Refs #729.
- **The process-group timeout test no longer fails against correct code.** `kill -0` succeeds for
  a **zombie** as well as a running process, so a reaped-but-uncollected descendant read as alive.
  It failed 5 runs in 20 on fixed `main`; the probe now also reads the process state. Refs #722.
- **The `grandchild_gh` stub no longer leaks a temp directory per run.** `#718` fixed the
  process-group kill and its new test helper created a `$TMPDIR` directory per call that nothing
  removed, reintroducing the `#713` leak one PR after that issue was closed. It accumulates
  separately from `#713`'s because the name prefix differs (`-grandchild-` vs `-test-`), and it
  fires on every run of the suite. The helper now uses the `TempDir` owner that `#713` added for
  exactly this shape, and a second control asserts the new call site, because the existing one
  covered only `fake_gh` -- which is why the leak passed a property that was already green.
  Refs #719.
- **Give the `gh api` call the 60-second timeout the reference has.** The reference bounds every call; the Rust port used `Command::output`, which waits forever, so a hung `gh` spends the whole `timeout-minutes: 5` job.
  It fails closed rather than wrongly, so the cost was availability, not correctness. A timeout
  kills the whole process group, not just the child, so helpers `gh` spawned do not survive
  holding the pipes. Refs #702.
- **Own the `src/bin` loopback port until the child that binds it is spawned.**
  `runtime_v3_game_information_entry_support.rs` drew its loopback address with a
  `free_loopback_address()` that read `local_addr()` and dropped the listener inside the same
  expression, so the port was unowned from that instant until the child bound it. That is the
  `#673` defect; `#693` fixed it in `tests/support/**` only, and deliberately did not reach
  `crates/harness/src/`.
  It is reachable end to end. `run_runtime_entry` puts the address in the owner config as
  `management_listen`, the runtime child binds it, and `wait_for_management` then polls
  `ManagementClient` until the route answers -- so a port taken in the window left the child's
  `bind` to fail with `EADDRINUSE` while the readiness check reported success, because it
  connects to whatever is listening rather than to the child.
  The two remaining call sites needed different fixes, and using one shape for both would have
  been wrong. `peer_encoding_tests.rs` rebinds in-process on the very next statement, so it now
  binds `:0` once and keeps that listener; there is no window to close and a reservation there
  would be pure ceremony. `entry_tests.rs` hands the address to a child, so it holds the listener
  in a `ReservedAddress` and releases it as the last statement before `Command::spawn()`.
  `release` is idempotent, which is what lets the replay pass rebind one address across two
  spawns without a separate flag. The allocator's prescribed fix -- returning the bound
  `TcpListener` -- is the same framing the `LivePeers` entry below records as measured and
  refuted on this kernel, so both sites take the reservation shape `#693` landed.
  The regression tests include `the_pre_fix_shape_really_does_leave_the_port_unowned` as a
  negative control, so the reservation test cannot pass for the wrong reason. No test here is
  known to fail this way and none is claimed to; the fix is on the verified code shape and the
  verified reachable call path. Refs #681, #673.
- **Stop the review gate's own test suite from failing on `ETXTBSY` while a stub is being
  written.**
  `review-of-record` is a required check on every pull request and it runs the gate's own tests, so
  a test-only race here blocked unrelated work at random — including after the author had already
  posted a correct review, which is the one signal that makes people re-run instead of read. Run
  `36386964129` failed on `tests::runner::head_sha_extracted_from_payload` with `Text file busy
  (os error 26)`, and run `36389325307` attempt 1 failed the same way on the same head that attempt
  2 and run `36389016026` passed: identical commit, opposite verdicts.
  The mechanism is **not** a path collision, and the per-call nonce `#700` added is not what was
  missing. `fs::write` closes its handle before returning, so nothing leaks: the window is the few
  microseconds between that close and `execve`, and `Command::output()` reaches the stub through
  `fork` + `exec`. If any *other* test thread is inside its own write-to-exec window at that
  instant, the forked child inherits that thread's still-open write descriptor, and the kernel
  refuses to `execve` an image any live descriptor holds open for writing. Each stub has its own
  directory and filename, so no per-call uniqueness can prevent it — the coupling is the inherited
  descriptor, not the name. Reproduced deterministically with distinct paths and distinct inodes:
  thread A holds its own stub open for writing, thread B forks 60 children on its own stub, and
  A's `execve` fails `ETXTBSY` 3 times in 3. That also makes the earlier diagnosis on the issue
  wrong in a way that matters: its supporting control — "8 threads rewriting and execing *their
  own distinct path*, 0 failures" — never put a `fork` inside another thread's write-to-exec
  window, so it reproduced the clean case and was read as the dirty one.
  This repository had already measured and fixed exactly this for its own written stubs:
  `tests/support/runtime_v4_executable_composition_process/spawn.rs` records **5 failures in 240
  spawns** without a retry and **0 in 240** with one. `tools/review-gate` never adopted it, which
  is why the one caller still exposed was the required check's own suite. The bounded retry is
  ported for the same measured reason, and `Command::output()`'s stdio wiring is reproduced
  explicitly, since a bare `spawn` would otherwise capture no output and fail every read closed.
  Retrying is sound because the offending descriptor is always closed by its owner, so the
  condition is transient and the deadline cannot be outlived by a persistent one.
  The regression manufactures the condition rather than looping the suite and hoping, which is what
  let this survive `#700`: that fix was real, but was verified with **1 failing run in 8 against
  the old code and 0 in 12 after**, a rate that cannot distinguish "fixed" from "almost never hit"
  — one failure in 40 full-suite runs here. The new test holds a write descriptor open on the stub
  it is about to exec, releases it the way a forked child would, and asserts the runner still
  succeeds; verified to fail against the pre-fix implementation. A companion control asserts an
  un-retried `Command` on the same held-open stub still fails with `ETXTBSY`, so the regression
  cannot pass by never provoking the condition.
  Refs #707, #668, #609, #700.
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
- **Run every test binary in the CI test step instead of stopping at the first failure.**
  `cargo test` executes each test target as a separate binary and aborts the whole invocation at
  the first failing one unless `--no-fail-fast` is passed, and this crate's integration tests are
  individual binaries that cargo runs alphabetically by filename. On the first attempt of
  `Continuous integration` run `36338651404` at `fd4dc88d`, a single failure in
  `jev_bridge_process.rs` (position 128) stopped the run after 128 of the 264 top-level test
  binaries, so 136 never ran — including `served_gateway_capture_drain.rs` (position 245),
  `runtime_v4_executable_composition.rs` (position 222) and `workflow_store.rs` (position 264, the
  last) — and the rerun then went green, certifying only that every binary *up to the first
  failure* passed. The step now passes `--no-fail-fast`, which adds no retry and hides nothing: a
  genuinely failing test still fails the step and the job on its own merits. A new check asserts
  the invariant rather than the flag text — any workspace-wide `cargo test` must carry the flag —
  and carries a vacuity guard so a sweep that matches nothing cannot report success. Refs #645.
- **Install the review gate in `sts2-harness`.** `CONTRIBUTING.md` says a green run does not
  substitute for review, but nothing here enforced it: every workflow this repository ran was a
  product or policy check, none of them a review gate, and merges landed here with no review of
  record. The gate exists and is reviewed in `AI-Ascension/.github` (`review-gate.yml` plus
  `tools/review_gate.py`), and until this change it was installed in exactly one repository in the
  campaign -- the one that ships no product code. `sts2-harness` is where the gate belongs: it
  carries the large majority of the campaign's merges, and the large majority of the ones that
  landed with no review of record.
  It is installed as a new Rust workspace tool, `tools/review-gate`, plus `review-gate.yml`, and
  not vendored verbatim: this repository's `LANG001` rule prohibits Python source and
  `repo-policy --strict` enforces it, so the reference could not be copied without breaking the
  build. The decision logic is a case-for-case port, and the ported suite pins every branch the
  reference's own 25 tests pin -- the head-pin rule, the `COMMENT`-state rule, the
  all-reviews-not-just-latest regression, and every fail-closed path.
  The workflow checks out the **default branch** and runs the gate from there, so a pull request
  cannot edit the gate into passing itself: the one property worth more than the convenience of
  testing the pull request's own copy. The two halves therefore had to land in order -- the tool
  first, then the workflow -- because a single combined pull request would have checked out a
  `main` holding neither the tool nor its tests, and would have failed for a reason that has
  nothing to do with review. The job also runs the gate's own tests from that same default-branch
  checkout, so the gate cannot be weakened on `main` without turning its own check red. Refs #668,
  #49.
  One test-harness defect found by CI while landing it, and fixed here rather than papered over:
  the fake `gh` the `runner` tests execute derived its temp path from a hash of the *body* it would
  print, so two tests that legitimately pass the same body -- `real_subprocess_roundtrip` and
  `head_sha_extracted_from_payload`, both `{"head": {"sha": HEAD}}` -- resolved to the same file.
  One `execve`d the stub while the other was still writing it and lost with `ETXTBSY` ("Text file
  busy"). The name is now unique per call from an atomic counter, so the outcome no longer depends
  on scheduling: reproduced 1 run in 8 against the old naming, 0 in 12 after.
  Refs #668, #49.
- **Assert that a required structural marker occurs exactly as often as policy declares it.**
  `check_required_preamble` walked the file with an ordered scan that stopped consuming markers once
  they were all satisfied, so a *second* copy of a marker past the last one was never examined. A
  changelog that repeated `## Unreleased` with the six preamble lines copied above it satisfied
  every assertion the rule made, which is how a duplicate shipped in #676 and survived #680: the
  first marker check tested presence and order, and neither of those is uniqueness. The scan now
  counts every occurrence of every marker and reports a `DOC003` finding when a file supplies more
  copies than `policy.toml` declares. The comparison is surplus over *declared*, not a hardcoded
  one, so a policy that deliberately declares a marker twice is still satisfied by two headings and
  still reports a third as surplus. The duplicated preamble and heading themselves are removed here,
  and the changelog's waiver count is restated to match the cleaned file. Closes #682.
- **Split the control authority under the production size limit instead of acknowledging the
  breach.** `context_control/state.rs` sat 171 nonblank lines over `rust_production_max` and was
  held there by a `policy.toml` exemption. The one `impl ControlAuthority` had grown four separable
  concerns: the gate transitions, operation admission and settlement, the durable journal
  boundary, the idempotency/event-ledger bookkeeping those transitions share, and the digests they
  depend on. They are now `state/gate.rs`, `state/operations.rs`, `state/journal.rs`,
  `state/ledger.rs`, and `state/digest.rs`, leaving `state.rs` as the authority's types and its
  remaining stop, boundary, and plan-admission transitions. The
  exemption is deleted rather than reworded, so the size rule measures these modules for real
  instead of waiving them — which removes the last exempted breach the repository carried. This
  completes the three splits that `#640` tracks. No public item was added, removed, or renamed; the
  split is verified method-for-method against the pre-split file. Closes #574.
- **Split the provider renderer under the production size limit instead of acknowledging the
  breach.** `context_control/render.rs` sat 199 nonblank lines over `rust_production_max` and was
  held there by a `policy.toml` exemption. The renderer carried four separable concerns that had
  grown into one file: budget accounting, admission of context items, Exo request conversion, and
  the shared Ollama projection. They are now `render/limits.rs`, `render/admission.rs`,
  `render/exo_request.rs`, and `render/ollama.rs`, leaving `render.rs` as the boundary that
  declares and re-exports them. No public item was added, removed, or renamed — the split is
  verified item-for-item against the pre-split file — and the exemption is deleted rather than
  reworded, so the size rule now measures these modules for real instead of waiving them. This is
  the same treatment `#643` gave the provider-session metadata store, and it removes the largest
  of the three breaches that `#640` tracks. Closes #572.
- **Stop requiring a waived breach to exist before the policy gate will pass.** The
  `repository_reports_its_waived_breaches` test exists so the `EXEMPTED` finding cannot be computed
  and then silently discarded — the `#569` defect. It asserted both that the reported count matches
  the recomputed one *and* that at least one `EXEMPTED SIZE001` line appears. Once the last size
  exemption was replaced by a real split, that second assertion failed on a repository that is now
  fully compliant: there was correctly nothing left to waive, and the gate reported it as an error.
  The counting assertion — the one that actually catches a discarded finding — is unchanged, and a
  new assertion now pins the zero case explicitly, so the test still fails if a waiver is dropped
  from the output and still fails if one is invented where none is owed. Refs #574.

- **Match `DOC003`'s structural markers as whole lines, in order, anchored to the file's opening
  line.** The rule tested each marker with `text.contains`, so a marker was satisfied by its own
  string appearing anywhere in the file — including inside prose describing it. `CHANGELOG.md`
  documents its `## Unreleased` heading in prose, so deleting the real heading while that sentence
  survived reported nothing: root measured `0 warning(s), 0 error(s)` and exit 0 on a `CHANGELOG.md`
  with the heading deleted and the nonblank line count held at 493, which is the exact false green
  the rule was added to end. Markers now match whole lines, are consumed in declared order so one
  repeated heading cannot stand in for the preamble, and the first marker must be the file's first
  nonblank line — which is what the doc comment on `check_required_preamble` already claimed and
  the substring test never enforced. Closes #620.

- **Refuse to ship a conflict marker, because deleting the one that reached `main` proved the gates
  would not.** A bare `=======` sat in this file at line 29, left by the `#601` merge as a
  three-way-resolution artifact with no `<<<<<<<` or `>>>>>>>` counterpart, and it survived a merge
  and a full round of green CI. It survived because both gates covering this file are shaped so that
  it passes: the size rule scored the removal as a *reduction* in nonblank lines, and `DOC003`
  asserts that markers are *present*, not that conflict debris is absent. #618 removed this
  instance incidentally, while re-resolving this file for an unrelated entry; deleting it fixes one
  occurrence and leaves the class open, because the next rebase across this boundary puts it back.
  The class is not hypothetical. Each instance is cited by the commit that *introduced* it, and
  each removal by the commit that removed it. No clause rests on a branch, a rebase, or a count, and
  every citation is a commit rather than a branch tip, because a tip can be closed or superseded
  while the prose still names it. `main` is the only ref whose state is asserted.
  `501a711`, the `#601` merge, introduced the line-29 artifact, reached `main`, and `06eba7e6`
  (`#618`) removed it; `8e3ffea` added a second at line 45, which no commit in its own history has
  removed and which is not on `main`; `696e56f` added one at line 60, which `cba9be8` removed; and
  `df9ef25` added one at line 100, which `94802b8` removed. Of the four, only `501a711` is on `main`.
  `repo-policy` gains
  `CONFLICT001`, which reports any tracked text file with a line *starting* with seven or more `<`,
  `=` or `>`, naming the path and the line number so review can act on it.

  The match is positional rather than a substring search, because `contains` would fire on prose
  that merely names a marker — the defect `#620` is fixing in `DOC003` — and an equals run inside a
  sentence is ordinary text. It is anchored at the start of a line rather than requiring the line to
  be *entirely* a run, and that is the difference between a gate that works and one that does not:
  git writes its opener and closer with a label attached (`<<<<<<< HEAD`, `>>>>>>> topic`), so a
  whole-line test would match only the bare separator — the one case that happened to exist here —
  and would then stay silent through every future rebase leaving a complete block behind.
  Indentation is not a marker, since a merge never indents one and an indented run is an indented
  code sample.

  Fenced code blocks are deliberately **not** excluded. Excluding them would exempt the
  documentation of this very defect while still letting a real artifact inside a fence through, and
  prose about a marker never puts one at column zero, so the common case is clean without the
  exclusion. A setext underline of seven or more is reported, because git's separator is exactly
  seven and any longer threshold would miss the defect; this repository uses ATX headings
  throughout, so nothing in the tree is near that boundary. Verified by reproduction in all three
  marker forms under `--strict`, not by construction alone. Refs #624, #625.
- **Assert the changelog's own structure, because a size gate cannot see the loss of it.** The
  `#606` merge resolved a `CHANGELOG.md` conflict by keeping the bullet list and dropping the title,
  the preamble and the `## Unreleased` heading, and every gate stayed green: all twelve deleted
  lines were nonblank, so the size check scored the deletion as a *reduction* and passed. The file
  stopped identifying itself, and with the preamble went the pointers to
  `docs/CHANGELOG-ARCHIVE.md` and the dated archives — leaving the active changelog
  structurally indistinguishable from an archive, under an exemption that still called it "the
  active changelog". #611 restored the text; this adds the check that was missing. `policy.toml`
  gains a `project.required_preambles` table mapping an exact path to the markers that file must
  carry, and `repo-policy` reports `DOC003` for any that are absent. It is deliberately not a size
  rule and deliberately not an exemption — a waiver is the mechanism that let the original defect
  hide, so the assertion is a hard error on an exempt file too. Verified by reproduction, not just
  by construction: with the preamble deleted, `repo-policy --strict` reports both missing markers;
  restored, it is clean. Three tests cover the present case, the exact regressed shape (a bare
  bullet list), and a file outside the table. Closes #614.
- **Pin the shared 8 MiB capture total across both served gateway pipes, with the truncation
  notice paid for out of it.** #559 bounded each served pipe separately (4 MiB per stream) and
  added an 8 MiB ceiling across the pair, but the two bounds were only ever verified apart, and
  the interaction is the part that can go quietly wrong: a pipe cut by the *shared* budget trims
  its own notice to fit and charges it against already-retained bytes, and a bug there produces a
  capture that announces itself while silently omitting evidence. This change adds the tests for
  that combination. A served gateway that floods **both** pipes past the per-stream ceiling is now
  driven end to end: the pair the harness hands back never exceeds `MAX_TOTAL_CAPTURE_BYTES`, each
  cut pipe carries a truncation notice, and a gateway that stays inside every ceiling carries
  none — so "a notice is always present" cannot pass. The accounting is pinned at the unit level
  too, with the mutations named in #567 each shown to fail: charging the notice after the total is
  computed, dropping the trim-back, and charging the shared total against bytes that were never
  retained. The first of those is unobservable in the combined-flood shape alone and is caught by
  the per-stream form instead, which is why both shapes are kept. No served capture's bound
  changes: the ceilings were merged in #559; this only proves they hold together.

- **Split `context_control/membership.rs` so it stops hiding a 21-line hard-limit breach behind an
  exemption that denied it.** The file measured **421** nonblank lines against
  `rust_production_max = 400`. Its `policy.toml` exemption stated that count accurately and then
  asserted the implementation "remains below the hard limit" — false, and a claim the size gate
  was in a position to refute and did not, because `size_findings` skips an exempt path *before*
  reading it, so the prose was the only place the breach was recorded. The
  effective set, its per-reference outcomes, the dispatch projection, the revalidation state and
  the typed error vocabulary now live in a sibling `membership_effective.rs`, following the same
  `#[path]` idiom the module already used for `membership_resolution.rs` and
  `membership_selector.rs`, at **274** and **167** nonblank lines — both inside the 400-line hard
  limit, so the exemption is **deleted** rather than reworded. This is a pure source move: every
  item is re-exported from the parent so its public path, serde attributes and schema strings are
  unchanged, and no behaviour differs. `repo-policy --strict` reports one fewer exempted breach
  (3, down from 4; #606 took the pre-#606 count of 5 to 4), and `membership.rs` is gone from that
  list. No production, protocol, or runtime effect.
- **Split the provider-session metadata store's filesystem boundary into three sibling
  modules, so the file that hid a 57-line hard-limit breach is gone rather than reworded.** `state_store.rs` measured
  **457** nonblank lines against `rust_production_max` of **400**, and its `policy.toml` exemption
  asserted that 457 "remains below the hard limit" — false on the direction, and invisible to the
  size gate, because `size_findings` skips an exempt path before reading it. #569's exemption
  verifier is now able to refute exactly that sentence. The file carried two separable concerns:
  the **read and path-safety** rules (open without following a symlink, reject a path that escapes
  its directory or has a permissive mode), the **durable write** (write a uniquely named temporary
  file, give it private permissions, flush, then rename over the target), and the **refusal
  vocabulary**. Those move to `state_store_io.rs`, `atomic_replace.rs` and `store_error.rs` under
  the existing `state_store/` directory, beside `owner_lease.rs`, and the parent keeps the
  envelope, key and scope handling. The result is **272** / **110** / **68** / **49** nonblank
  lines — all inside both the 400 hard limit and the 300 preferred limit — and the exemption is
  **deleted**, not corrected: rewording the count instead of splitting the file is the exact
  failure this entry describes. This is a pure source move, checked mechanically against `main`:
  all **32** items the original file declared are present after the split, with none added and none
  dropped. No behaviour change — the `unix` and non-`unix` variants of each platform-sensitive
  helper move together, `owner_lease.rs` keeps its existing `super::` imports through a re-export,
  and the error enum is re-exported from the parent so `ProviderSessionMetadataStoreError` keeps
  its public path. Closes #571.

- **Make the management client's header allow-list guard non-vacuous, and correct the criteria
  that produced it.** The guard added for the `Accept` fix transcribed the gateway's
  `header_is_allowed` exactly, then added a nineteenth entry, `idempotency-key`, under the comment
  "Admitted by the gateway as a non-gateway header" — a category that does not exist, since
  `header_is_allowed` is a bare `matches!` with no exemption clause. Separately, the test only ever
  built a head with the idempotency argument `None`, and the header is emitted only in the `Some`
  branch, so the entry could neither fail nor be reached: the test asserted over a request that
  never carried the header it existed to police. Measured, that nineteenth entry was the sole
  difference between the guard passing and failing, so a head the gateway refuses with
  `400 unsupported_header` was asserted as safe. The fabricated entry and its false comment are
  removed; the plain head is now asserted against the gateway list outright, and the idempotent
  head is pinned to differ from that list by *exactly* `idempotency-key` — which fails both if a
  new unadmitted header appears and if the header is ever dropped. A new test asserts the guard is
  non-vacuous, which is the check whose absence let the defect through. The header is deliberately
  **not** removed: it is required by the management server (`idempotency_key_required`) and declared
  `required: true` on 16 operations across the memory and control OpenAPI contracts, so dropping it
  would turn 16 policy mutations into `400`s. The documented invariant is the precise one — every
  header this client sends is admitted by the server it is pointed at, which today is the harness
  management listener, not the gateway. Refs #598, #560.

- **Split the synthetic management adapter so it stops hiding a 531-line hard-limit breach behind a
  false exemption.** `management/workflow_ports.rs` measured **931** nonblank lines against a
  `rust_max` of **400** — more than double — and its `policy.toml` exemption claimed the file
  "remain[s] below the hard limit". Because `size_findings` skips exempt paths *before* reading
  them, the stated count is the only assertion of that count anywhere, so nothing prompted anyone
  to check it and the breach stayed green. The file is now nine modules, largest **293** nonblank,
  all inside the 400-line hard limit, and the exemption is **deleted** rather than reworded. The
  modules group along their own seams — definition admission, context inspection, capability
  reporting, in-memory and persistent execution, replay — plus a small shared support module for
  error translation, with the three fixture constructors left in `workflow_ports.rs`. The split is
  behaviour-preserving: every one of the 931 original nonblank body lines survives, modulo the
  deliberate `pub(super)` visibility the module boundaries require and the re-wrapping of the
  original single `use` block into eight per-module ones. Splitting a file is exactly the kind of
  change that silently drops an import or a visibility edge, so the split was verified by compiling
  rather than by inspection: `cargo check -p sts2-harness --lib` passes with **0 errors and 0
  warnings**, and `repo-policy --strict` reports **0 warnings, 0 errors**. Refs #570.

- **Split the provider-session policy HTTP suite so it stops hiding a 114-line hard-limit breach
  behind a false exemption count.** `provider_session_policy_http.rs` measured **714** nonblank
  lines against a `rust_test_max` of **600**, and its `policy.toml` exemption claimed **557** and
  that the file "remain[s] below the 600-line test hard limit". The size gate cannot catch this on
  its own: `size_findings` skips exempt paths *before* reading them, so an exemption's stated count
  is the only assertion of that count anywhere. Because the prose reads as a durable, reviewed
  justification, nothing prompted anyone to check it, and the breach stayed green. The suite is
  now three files — the redacted read/reopen projection, adoption identity and admitted-run
  binding, and the command lifecycle covering CAS, idempotency, restart and control grants — over a
  shared `support/` fixture module, at **133 / 171 / 288** and **171** nonblank lines, all inside
  the 400-line preferred budget, and the exemption is **deleted** rather than reworded: correcting
  the count instead of splitting the file is the exact failure mode this entry describes. All ten
  original tests are preserved and still pass, unchanged in behaviour; this is a source layout
  change only, with no production, protocol, or runtime effect. The one-string fix from the same
  family is included: `served_gateway_evidence_naming.rs` named a graph lane `graph-original`
  where the real request ids are `graph-changed` and `graph-base`, so the fixture was pinned to an
  id that does not exist — harmless today because that test only asserts pairwise distinctness,
  and silently rot for the same reason this exemption did. Refs #564.
- **Retire five unreachable Rust sources and gate the whole class.** `#491` found five tracked `.rs`
  files that no crate root reached, so they never compiled and their tests never ran. Four are
  superseded duplicates: `runtime_v3_episode_actions.rs` against the `include!`d
  `runtime_v3_episode_helpers.rs` (whose `retain_operation` is stricter, including the payload
  check), `runtime_v3_lifecycle_reconnect_test.rs` against the recovered reconnect test, and the
  `#220` residue `policy_owner/owner_impl.rs`/`change.rs`. The fifth, `sts2-astra-bridge_tests.rs`,
  held one assertion with no live counterpart, now ported into `sts2_astra_bridge_tests.rs`.
  `repo-policy` enforces `RUST002`: a tracked `.rs` inside a compiled crate that no crate root
  reaches through `mod`, `#[path]`, `#[cfg_attr(..., path = ...)]`, or `include!` now fails
  `--strict`, so a lost `mod` line turns a check red instead of silently dropping coverage. No
  runtime, provider, game, or native behavior changes. Closes #491.

- **Extend the real pinned-Exo CI lane with the `#148` fault and isolation matrix.** The landed lane
  executed the real Exo process oracle but exercised only a few admission rejections. A new
  `fault_oracle` test proves the admission faults fail closed with zero model egress (config schema,
  extension/node/executor pins, relative executor path, argv config identity, provider-route
  refusal), that a model endpoint which consumes the request and then closes with no reply fails the
  run within the bounded process lifetime, and that two sequential or concurrent runs each send
  exactly one model request with no shared endpoint, temporary or state root (`#148` T3). The lane
  writes a bounded `target/exo-fault-report.json` and asserts its revision against
  `EXO_SOURCE_REVISION`. This is real-process evidence with a synthetic model and synthetic host;
  live provider, game and native acceptance remain separate. Refs #148.

- **Pin the authenticated-request constructor so the guard cannot silently stop naming it.** The
  fence pair added for `#481` cannot detect a *rename* of `from_transport`: renaming it while it
  stays `pub(crate)` leaves both fences green — the `compile_fail` snippet now dies of `E0599`,
  which the inert `,E0624` clause ignores, and the compiling companion pins only the type paths and
  `WorkerCapability::Dispatch`. Neither doc fence can close this by construction, because both
  compile as an *external* crate and can never name a `pub(crate)` item. An in-crate
  `#[cfg(test)]` assertion now pins the constructor's name and signature at its `pub(crate)` path;
  it runs under the existing `Run Rust tests` target (`cargo test --lib`/`--all-targets`), which is
  different from the `Run doctests` step, and fails to compile if the constructor is renamed or its
  signature changes. Compatibility: test-only; no production code, schema, route or behavior
  change. Refs #485.

- **Gate Exo compatibility with the real pinned Exo process in CI.** No workflow executed the
  repository's own Exo bridge lane: `experiments/exo-agent/bridge` is `[workspace]`-excluded, so
  `cargo test --workspace` could not reach the `#[ignore]`d `process_oracle`/`lookup_oracle` tests,
  and the pinned `exoharness/exo` checkout in `ci.yml` fed only lifecycle fixtures. A new
  `exo-process-oracle.yml` installs the pinned Node/pnpm, stages the owned extension into the pinned
  read-only Exo checkout, builds the isolated `sts2-exo-executor` and `sts2-exo-bridge`, and runs both
  oracles against the real Exo TypeScript runtime with a synthetic loopback model and synthetic host.
  The `EXO_SOURCE_REVISION` pin is re-read at run time and the bounded evidence report is asserted to
  name it. This proves process composition only; live provider, game and native acceptance remain
  separate. Refs #148.

- **Make the doctest gate's guarded property actually protected.** `#480` began executing the
  workspace's doctest, but its `compile_fail,E0624` annotation does not enforce the error code. On
  rustdoc 1.97.1 a snippet that dies of `E0432` (unresolved import) or `E0425` (undeclared name)
  still reports `ok`, and an unknown code such as `E9999` is accepted silently, so only
  "compilation fails for some reason" was ever asserted. The fence reaches the `pub(crate)`
  constructor through four public re-exports, so removing any one of them would have left CI green
  while the assertion stopped testing the constructor at all — the same class the parent gate was
  added to prevent, one level up. The doc comment now carries a second, **compiling** fence that
  pins those same paths and turns red the moment one is renamed or removed; the `compile_fail`
  fence is left to assert the authority property it can actually assert. Compatibility: docs and
  doctest only; no production code, schema, route or behavior change. Refs #481.

- **Run the workspace doctests in CI.** No gate executed doctests: the `rust` job runs
  `cargo test --workspace --all-targets --all-features --locked`, and `--all-targets` excludes the
  `--doc` target by definition, so the repository's single doctest — the `compile_fail,E0624` guard
  proving that an external caller cannot construct an `AuthenticatedWorkerRequest` — had never been
  compiled in CI. A new `Run doctests` step runs `cargo test --workspace --doc --all-features
  --locked`, so that authority-boundary assertion (and any future doctest) is now executed and
  cannot rot silently. Compatibility: CI-only; no source, schema, route or behavior change.
  Refs #479.

- **Repair nine intra-doc-link defects and gate the class durably.** `sts2-harness` failed a
  documentation-integrity expectation its own gates could not see. Seven intra-doc links across six
  files named a type or method that does not resolve at the file's own scope — three of them
  reachable by a plain `cargo doc` — and two more named a bare `[`plan`]`, ambiguous between a
  function and a module (`benchmark_manifest::branch_experiment` and `benchmark_manifest::suite`).
  Every one names a real item elsewhere in the crate, so each was a scope/path or disambiguation
  defect rather than a stale name: `execution::types::worker` looked for `ExecutionStore` under
  `execution::types`, which re-exports only `ExecutionStoreError`;
  `management::save_profile_setup::setup` attributed `verify` to `VerifiedProfileReadback` when it is
  an inherent method of `ProfileReadback`; `provider_session::types::effective_limits` linked a bare
  `ProviderSessionPolicy`; and `context_control::membership`, `context_control::model_view` and
  `management::lifecycle` linked bare names owned by sibling modules. Each link now carries a path —
  or, for the two ambiguous `plan` links, a `()` disambiguator — that resolves. The durable half is a
  `cargo doc` step in the `rust` job with
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links --document-private-items"`, because a default
  rustdoc run skips the private modules that hold four of the seven unresolved links; no workflow had
  run `cargo doc`/`rustdoc` before, so nothing owned the class. Refs #477.

- **Keep the rejected output name out of the recipe refusal.** The pre-agent recipe admission
  contract documents that its refusal vocabulary "carries only structural identity, never a supplied
  argument value or game text", and every variant met that except one:
  `RecipeAdmissionError::InvalidOutput` stored the raw `OutputSlot.name` that had just failed
  `is_identifier` — by construction a value guaranteed to violate the ≤96-byte, `[A-Za-z0-9._:-]`
  bound, and free to carry control bytes or arbitrary authored text. It now reports the offending
  slot **index** instead, matching how the other variants are built (the step and dependency fields
  are typed identifiers; `DuplicateOutput.output` passed its shape check). Reachability is
  Rust-API-only today — `RecipeDefinition`/`OutputSlot` have no `serde` intake and the module has no
  consumer outside `recipe/` and its test — so nothing untrusted could reach the error yet; the
  exposure would have begun when T2/T3 add authored or Studio-facing intake. Compatibility:
  safety-correction — the variant is public but the crate is consumed only by its own workspace, no
  record/schema/route/digest changes, and which recipe is refused (and at which point in the fixed
  admission order) is unchanged. Refs #97; see
  [ADR 0073](docs/decisions/0073-pre-agent-read-only-recipe-admission.md).

- **Correct the bounded-analysis documentation contract.** The `workflow::bounded_region` module
  doc cited a `BoundedAnalysis` type defined on no revision across all 64 remote refs and the bare
  `[`AnalysisValue`]` beside it — both dangling intra-doc links — and claimed a declared adaptive
  region now runs "on the budget-reserved bounded route instead of only through a caller-supplied
  adaptive executor", which the shipped wiring does not do: `DynamicRuntime::step()` still
  dispatches a declared `adaptive_region` node to `DynamicExecutorPort::execute_adaptive`, and
  `execute_bounded_region` has exactly one caller, a test. No gate saw it — no workflow runs
  `cargo doc`/`rustdoc`, the crate denies no `rustdoc::broken_intra_doc_links`, and the private
  module is skipped by a default rustdoc run — so it built, linted and tested green while its own
  contract statement was false. The module doc now names the real entry point
  (`DynamicRuntime::execute_bounded_region`) and its inputs, states the node route is unchanged and
  the bounded route has no in-repo production caller, and records the two bindings the route
  deliberately does not make: base-revision continuity, and the caller-supplied region against the
  workflow's declared `adaptive_region` node, which no accessor exposes (the #465 review left it a
  design extension). The public `execute_bounded_region` rustdoc is corrected to match — the region
  is *caller-supplied* and is checked against the plan
  (`BoundedRegionRefusal::PlanIdentityMismatch`) but never a declared node. Compatibility: no code,
  schema, route, refusal, bound or digest change; documentation only. Source-only: no native
  effect. Refs #470.

- **Bind bounded-region admission to the plan's region and planner-profile identity.** A
  follow-up review of the bounded parallel analysis route found that
  `workflow::bounded_region::admit_bounded_region` checked the parallel cap, the region's
  admissible operations and the plan's structural validity but never compared the plan's
  `region_id` / `planner_profile_ref` against the region it was admitted for, unlike the sibling
  `DynamicPlanRegistry::accept`. Because both the plan and the region are caller-supplied, a plan
  that named a different region or planner profile was admitted whenever its operations fell inside
  the caller-supplied `allowed_operations`. Admission now refuses such a plan with a dedicated typed
  reason, `BoundedRegionRefusal::PlanIdentityMismatch`, before any branch is dispatched, and the
  module contract states that base-revision continuity remains `accept`'s responsibility because the
  region does not carry the base digest or revision and the runtime supplies only the workflow
  limits. Compatibility: tightening — this route has no in-repo caller, and a plan that names its own
  region and profile is unaffected. Source-only: no native effect. Refs #465.

- **Hold the Exo request-level identity to the published wire width.** The published
  `sts2.exo-bridge-wire-v1` schema binds `decision_request.model_execution_id` and `.state_id` to
  `$defs/id` (`maxLength` 512) and the protocol validator admits the same, but the lifecycle
  manifest refused both at 128, and the internal identities the owner mints from them
  (`lifecycle-binding-`, `lifecycle-prepared-`, `provider-execution-`) were refused above an
  effective 109 bytes — a ceiling written in no schema. The two request-level fields are now
  validated against the published width while every envelope/control identity keeps its own
  128-byte bound, so a host that follows the published schema is no longer refused before
  dispatch, and the two refusal vocabularies that depended on how wide the value was are gone.
  Internal identities carry a digest of the request identity rather than the identity itself, so
  their width no longer grows with it. Compatibility: additive at the wire — it only admits
  identities that were previously refused and changes no published schema; every refusal stays
  fail-closed before dispatch. Refs #458; see
  [ADR 0077](docs/decisions/0077-exo-request-identity-width.md).

- **Execute a bounded parallel analysis region through the production dynamic runtime.** The
  budget-reserved bounded route (`execute_plan_bounded`,
  `execute_plan_bounded_reserved`) had no production caller: a `DynamicRuntime` handled an
  `adaptive_region` node purely by delegating to the caller-supplied executor, so the owner's
  parallel cap, the atomic budget reservation and the per-branch join report were reachable only
  from tests. A new `workflow::bounded_region` module admits a declared region fail-closed
  *before* any branch is dispatched (cap from the workflow's own limits, admissible operations,
  then `validate_plan`), runs it on the reserved route, and reports a versioned, digest-bound
  outcome (`ascension.harness.bounded-analysis-report.v1`) that names each branch's actual joined
  state — settled, failed with its own reason token, or unknown — instead of a settled count, so
  a failed or lost branch can never be read as a success. `DynamicRuntime::execute_bounded_region`
  retains that report for a consumer; the existing node route is unchanged. The mutation clause
  holds by construction rather than by assertion: `DynamicNodeKind` is a `deny_unknown_fields`
  `analyze`/`decide` enum and the executor returns an `AnalysisValue`, so a plan naming a mutating
  node kind is refused at decode and no bounded branch can reach a game mutation. Compatibility:
  additive; two new module files, one new runtime method, no change to an existing schema, route,
  digest or node kind. Refs #98.

- **Admit explicitly scoped research inspection of hidden checkpoint state.** A new
  `research_inspection` module fixes the source-only contract behind #129 and separates privileged
  research data from the ordinary player-visible boundary: an operator-supplied grant binds one exact
  checkpoint, run, branch and consumer lane to an explicit, bounded set of field groups, so a
  gameplay lane cannot escalate by asking for a different visibility parameter, and revocation is
  monotonic so a replayed request cannot outlive its approval. Fields come from a closed matrix whose
  references refuse paths, queries and unbounded names; admission returns the admitted slice rather
  than fabricating availability, and the native owner's report must match it field-for-field and
  in order. Coverage stays distinct — `NotMaterialized`, `SimulationRequired` and `Unsupported` never
  collapse into zero, empty or an invented value — refusals carry no value, field name or digest, and
  paging is bounded so a partial page is never labelled complete
  ([ADR 0076](docs/decisions/0076-scoped-research-inspection-of-hidden-checkpoint-state.md)). The
  native capture read adapter and the capture-manifest agreement remain open. Refs #129.

- **Refuse a suite whose case and policy axes cannot derive a settleable trial key.** `SuiteManifest`
  bounded a `case_id` and a `policy_id` separately at `MAX_SUITE_LABEL_BYTES` (128), while
  `TrialOutcome::validate` refuses a `trial_key` over `MAX_TRIAL_KEY_BYTES` (256) and the key
  concatenates both labels around a 64-hex suite revision. A manifest that validated could therefore
  plan a trial whose outcome `settle` refused forever. The combined pair is now bounded by a derived
  `MAX_SUITE_TRIAL_AXIS_BYTES`, so every accepted manifest is plan-and-settleable, and an oversized
  single id is still refused as an invalid label. Source-only: no released artifact was affected and
  no live caller reached the case. Compatibility: an input that previously validated and then failed
  at settlement is now refused at validation.

- **Bind readiness settlement to its proof, and let a starved wait expire.** The
  `management::readiness_wait` contract behind #96 now admits one `MilestoneObservation`, which binds
  the sealed owner readiness proof to the milestone and process generation that owner reported, so
  the facts that decide settlement are no longer separate `observe` arguments that one owner's proof
  could be paired with a milestone nobody reported. `ReadinessWait::expire_if_elapsed` advances the
  bounded clock without an observation, so a wait that stays starved times out instead of staying
  open forever, and an expired wait can never be satisfied afterwards. Compatibility: additive to the
  milestone vocabulary, the versioned target and the refusal vocabulary; the Studio round-trip and
  the native loading check remain open. Refs #96.

- **Map save-profile setup through a capability-gated operation contract.** A new
  `management::save_profile_setup` module fixes the source-only contract behind #102: authored
  discovery, selection and provisioning map one-to-one onto the accepted MCP tools and fixed
  gateway routes, with separate grants, a closed versioned request whose identities refuse paths
  and URLs, effect-free discovery, selection fenced by a required baseline whose identity is the
  owner's baseline identity and is independent of the selected slot, provisioning that cannot
  fence a baseline it does not yet have, and a readback that must match the admitted identity
  before downstream setup progresses
  ([ADR 0075](docs/decisions/0075-capability-gated-save-profile-setup-mapping.md)). The durable
  adapter, the boundary validation matrix and every real profile mutation remain open. Refs #102.

- **Wait for an identity-bound readiness milestone.** A new
  `management::readiness_wait` module fixes the source-only contract behind #96: an authored
  workflow names a versioned milestone target with a bounded deadline and attempt budget, and a
  per-generation wait settles only from fresh authoritative evidence bound to the same instance,
  authority epoch and process generation, with distinguishable timeout, denial, cancellation and
  restart-invalidation outcomes and stale or foreign evidence refused
  ([ADR 0074](docs/decisions/0074-identity-bound-readiness-wait.md)). The Studio round-trip and the
  native loading verification remain open. Refs #96.

- **Admit a bounded pre-agent read-only recipe.** A new `recipe` module fixes the source-only
  contract behind #97: an authored workflow may declare a bounded, versioned recipe of approved
  read-only tool reads that the harness admits before provider dispatch, with a fixed refusal order,
  a declared topological step order with no cycles or forward references, and mutation tools refused
  from the read-only catalog ([ADR 0073](docs/decisions/0073-pre-agent-read-only-recipe-admission.md)).
  Collection execution, provenance and the Studio round-trip remain open. Refs #97.

- **Plan, schedule and compare bounded same-start branch experiments.** A new
  `benchmark_manifest::branch_experiment` module fixes the effect-free contract behind issue #119: a
  versioned declaration of one verified fork point, a fork strategy, child policies and per-child and
  total budgets; a stable per-child trial key with its own fresh provider/context namespace; a
  same-start admission re-check that keeps a prefix-only start out of exact-restore statistics; a
  retry-safe recorded scheduler that reconciles a lost reply without double-scoring a trial; an
  aligned comparison that separates declared policy divergence from restore failure and does not let
  an identical endpoint erase an earlier divergence; and a sanitized report carrying a keyed handle
  and no exact digest. Source-only
  ([ADR 0072](docs/decisions/0072-branch-experiment-comparison.md)): the live children, the restore
  and the provider calls stay with the gateway and game-mod. Refs #119.

- **Resolve and durably bind an authored workflow's seed.** A new `seed_binding` module fixes the
  source-only contract behind #103: an explicit or generate-once seed normalizes to one bounded
  canonical UTF-8 form, a generate-once run draws at most once per persisted record and the persisted
  effective seed is reused across duplicate requests, lost responses and restarts without a redraw
  (only a retry after a failed persist may redraw, before any record exists), the effective seed is
  persisted before any setup mutation (failing closed), and a wrong instance, stale baseline or lease,
  unsupported setup, or conflicting persisted seed is refused before any draw. A recording transport proves the
  persisted effective seed and operation identity are sent unchanged. Native seed acceptance stays
  gated by sts2-game-mod#79
  ([ADR 0071](docs/decisions/0071-authored-seed-binding.md)). Refs #103.

- **Bind a benchmark rerun admission to the exact declaration it compared equal.** `RerunAdmission`
  now owns the admitted `Manifest`, reachable only through `RerunAdmission::declaration()`, so a
  `RerunAllocationSeam` cannot allocate for a declaration other than the one whose controlled inputs
  compared equal. Source-only contract tightening for #121; the equal path is unchanged.
  Refs #121.

- **Make the served capture surface configured and fail-closed.** Which sink the served composition
  attaches and what it retains is now an owner decision recorded in
  [ADR 0070](docs/decisions/0070-served-capture-configuration-and-retention.md): an unset surface
  keeps the merged in-memory recording ring, `metadata` and `off` are selectable, and every
  contradictory or out-of-range `STS2_WORKFLOW_CAPTURE_*` value is refused at startup rather than
  silently downgraded. Restart-durable capture bytes and the unrecorded Ollama `HttpBody` boundary
  remain accountable residuals (#145). Compatibility: no change to the served default behaviour.
  Refs #398.

- **Admit alternative gameplay forks from verified seeded replay prefixes.** A new
  `benchmark_manifest::prefix_fork` module fixes the effect-free fork-admission contract behind
  #117: an exact seed/profile/build/compatibility binding, a settled nonterminal boundary with
  complete receipts and one resolved legal action, a zero-provider-call replay, bounded sibling
  forks with distinct identities, and a forward-only replay-to-child handoff that reconciles a lost
  target. Source-only ([ADR 0069](docs/decisions/0069-prefix-fork-admission.md)). Refs #117.

- **Orchestrate isolated cold-launch benchmark trials from one pristine baseline.** A new
  `benchmark_manifest::cold_launch` module fixes the per-trial isolation contract behind issue #122:
  an immutable baseline binding the artifact digest, launch profile and closed telemetry exclusions;
  an exclusive, bounded destination lease; an opaque gateway-attested process birth with its own
  instance generation, since a PID can be reused; a readiness proof bound to that birth; a recorded
  stage machine that reconciles a lost reply without adopting another trial's state and quarantines
  an uncertain destination; and machine-readable cold-start evidence whose cleanup failure is
  distinct from the gameplay outcome. Source-only: native process evidence and the real child-process
  lane stay gated by sts2-game-mod#79
  ([ADR 0068](docs/decisions/0068-cold-launch-trial-isolation.md)). Refs #122.

- **Plan, schedule and report reproducible multi-policy benchmark suites.** A new
  `benchmark_manifest::suite` module freezes an ordered seed corpus, policy axis, repetition count,
  evaluator revision, declared budgets and predeclared metrics under a versioned manifest; plans one
  stable logical trial per suite revision/case/policy/repetition with its own provider/context
  namespace; keeps retry-safe attempt lineage so a replayed settlement is idempotent and a conflicting
  one is refused; preserves attempt counts across resume; and exports a sanitized aggregate with
  explicit denominators, honest paired comparisons and metric availability, never counting an
  infrastructure failure as a defeat, an unavailable cost as zero, or an unverified start inside an
  exact-start group. Source-only: native exact-start certification stays gated by #126
  ([ADR 0067](docs/decisions/0067-reproducible-benchmark-suite-scheduling-and-reports.md)). Refs #125.

- **Add offline trace-bundle admission and a bounded reproducer for divergence diagnosis.** A new
  `trace_divergence` module derives an immutable `TraceBundleManifest` per bundle, admits two bundles
  by closure, profile and action-schema coverage *before* comparing, compares bounded record views,
  reports explicit record/entry/byte truncation, and exports a `ReproducerPrefix` that replays only
  up to the failing boundary and validates against the original source. Offline and read-only; the
  public status stays digest-free. Native mismatch validation remains gated by #123
  ([ADR 0066](docs/decisions/0066-offline-trace-bundle-admission-and-reproducer.md)). Refs #124.

- **Record the two source-only Jev-runner decisions.** The wall-clock-sensitive global-time-budget
  test's fixture strategy is recorded in
  [ADR 0063](docs/decisions/0063-jev-runner-first-arm-admission.md): the first scheduled arm is
  admitted structurally rather than by fixture timing (#388). The 1,000 ms teardown cleanup bound is
  accepted as a host-load-dependent contract in
  [ADR 0064](docs/decisions/0064-jev-runner-teardown-cleanup-bound.md), with the strict closure
  assertion intact and the sampling limitations retained (#394). The Jev-evaluation Node suite was
  rerun without retry masking; first-attempt results are in
  [the no-retry matrix evidence](docs/evidence/jev-evaluation-noretry-matrix-20260923.md).
  Compatibility: documentation only; no code, record shape, or runner contract change.
  Refs #388, #394.

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
