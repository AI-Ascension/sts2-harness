# Changelog archive: 2026-10-08

These completed entries were moved verbatim from [`CHANGELOG.md`](../CHANGELOG.md) to keep the
active changelog within its preferred size budget. No relative links in the moved entries needed
rebasing.

### Archived from CHANGELOG.md

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
