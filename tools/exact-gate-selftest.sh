#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Regression tests for `exact-gate.sh`'s executed/passed accounting
# (sts2-harness#540).
#
# The gate exists to refuse a lane step whose `--exact` filter matched nothing.
# It used to count `test result: ok. 1 passed;` lines, which has two wrong
# answers: a test that RAN and FAILED counted as zero executions, and one
# invocation naming two `--exact` filters could never be satisfied. Both are
# regressions against the protection the gate is supposed to provide, and both
# are silent -- the gate is only ever read when a lane is already red.
#
# Every case below runs the real gate over a synthetic libtest log written in the
# exact shape libtest emits, so a change to the awk expression is what is under
# test here, not a re-implementation of it.
#
# usage: exact-gate-selftest.sh [path-to-gate]
set -uo pipefail

gate="${1:-$(cd "$(dirname "$0")" && pwd)/exact-gate.sh}"
tmp=$(mktemp -d) || exit 2
trap 'find "$tmp" -mindepth 1 -delete; rmdir "$tmp" 2>/dev/null' EXIT

passed=0
failed=0

# run_case <expected-exit> <expected-executed> <label> <log-body>
#
# The wrapped command is a tiny emitter (`sh -c 'cat "$1" --'` plus a literal
# `--exact` token in the argument list) rather than `cat <log> --exact`, because
# `cat` rejects `--exact` as an unrecognized option. The gate counts `--exact`
# tokens in the guarded command's own argv, so the token has to reach that argv
# while the command still succeeds; separating the two keeps every case below a
# genuine pass/fail of the executed count rather than an artifact of the wrapper.
#
# The body is written to a SOURCE file distinct from the gate's log path. The
# gate runs its guarded command with `>"$log"`, so an emitter reading the log
# path would read the file the shell is midway through truncating and would see
# an empty run in EVERY case -- which is the very shape case 3 asserts, so the
# suite would have passed for the wrong reason.
# run_case <expected-exit> <expected-executed> <label> <named-filters> <log-body>
run_case() {
    expected_exit=$1
    expected_executed=$2
    label=$3
    filters=$4
    log="$tmp/$label.log"
    src="$tmp/$label.src"
    printf '%s' "$5" >"$src"

    out=$(
        "$gate" "$log" sh -c 'cat "$1" --' sh "$src" $filters 2>&1
    )
    status=$?

# The counter is read under EITHER label -- `executed=` on the current gate,
# `matched=` on the pre-#540 one. Accepting both keeps every case below a test of
# the gate's ACCOUNTING rather than of its output wording, so this suite fails
# against the old gate for the substantive reason (it counts passes, and it
# cannot be satisfied by a batched invocation) and not merely because a string
# was renamed.
    actual_executed=$(
        printf '%s\n' "$out" |
            sed -n 's/.*exact-gate: .*\(executed\|matched\)=\([0-9]*\).*/\2/p' |
            tail -1
    )
    actual_executed=${actual_executed:-MISSING}

    if [ "$status" -eq "$expected_exit" ] && [ "$actual_executed" = "$expected_executed" ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        printf '#   expected exit=%s executed=%s\n' "$expected_exit" "$expected_executed"
        printf '#   actual   exit=%s executed=%s\n' "$status" "$actual_executed"
        printf '%s\n' "$out" | sed 's/^/#   /'
    fi
}

# A filtered single test that passes: one execution, gate accepts.
run_case 0 1 pass_single '--exact some_named_test' \
    'running 1 test
test some_named_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 1.59s
'

# A filtered single test that RAN AND FAILED. Under the old `ok. 1 passed` pattern
# this read as 0 executions, and the gate then blamed a renamed or removed test
# for a run that had actually executed and failed. What is under test is the
# executed count (1, not 0) and the diagnosis: this case emits no non-zero exit,
# so the gate is judged on its counters and its message, not on a red step.
run_case 0 1 fail_single '--exact some_named_test' \
    'running 1 test
test some_named_test ... FAILED

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
'

# The case the gate was written for: the filter matched nothing, the command
# still exits 0, and the gate must refuse.
run_case 1 0 empty_run '--exact some_renamed_test' \
    'running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.00s
'

# One invocation naming two `--exact` filters emits a single `ok. 2 passed;`
# line plus one per-test line each. Under the old `ok. 1 passed` pattern this was
# unsatisfiable, because no line in the log matched.
run_case 0 2 two_filters '--exact first_named_test --exact second_named_test' \
    'running 2 tests
test first_named_test ... ok
test second_named_test ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.12s
'

# A terse/--quiet lane prints the summary but no per-test lines. The summary
# totals are the only executions available, and they must still be counted.
run_case 0 2 terse_summary '--exact first_named_test --exact second_named_test' \
    'running 2 tests

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.12s
'

printf 'exact-gate-selftest: %d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
