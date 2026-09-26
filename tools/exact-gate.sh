#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Fail a lane step when a `cargo test ... -- --ignored --exact <name>` invocation
# does not actually execute the test it names (sts2-harness#524).
#
# libtest exits 0 while printing `0 passed; N filtered out` when a filter matches
# nothing, so the command's exit status cannot distinguish "the named test ran and
# passed" from "the named test was renamed or removed". Every harness lane that
# filters a test by name therefore reported green for an empty run. This wrapper
# captures the output and asserts the match count the exit status cannot see.
#
# The named count is the number of `--exact` tokens, not the token after the last
# one, because the lanes pass the filter both as `-- --ignored --exact <name>` and
# as `<name> -- --ignored --exact --nocapture`. The executed count is the number of
# libtest result lines whose `passed`+`failed` count is at least one, which is what
# distinguishes "the named tests ran" from "the filter matched nothing".
#
# It counts EXECUTIONS, not passes. A test that ran and FAILED is an execution: the
# gate must still refuse it (the command's own exit status already does) but it must
# not report `0 executed`, which would be false -- the run was not empty, and it did
# not pass (sts2-harness#540). Counting `test result: ok. 1 passed;` also made a
# single invocation naming several `--exact` filters unsatisfiable, because one
# `cargo test` invocation emitting `ok. 2 passed;` matches no such line; the
# result-line form counts those executions too, so batching filters now works.
#
# It deliberately does not use a pipeline: every lane invocation it wraps is the
# last command of its step, so the shell's `errexit` sees the gate's own status.
#
# Every harness lane checks this repository out at `path: harness`, and the
# evidence steps run with `working-directory: harness`, so the gate is addressed
# by its checkout-rooted path there (`$GITHUB_WORKSPACE/harness/tools/...`)
# rather than the repository-relative one.
#
# Known limits, so a later reader does not over-read a green gate:
#   - a multi-invocation command is checked for *at least* the named count; which
#     of several names matched is not identified;
#   - a command whose `--exact` filters legitimately execute zero tests cannot be
#     distinguished from a renamed one, and is rejected rather than accepted.
#   - the guard refuses when the guarded command reads the log path itself (the
#     shell has already truncated it, so the log is empty). This fails CLOSED --
#     a red lane, never a false green -- and no lane in this repository does it:
#     all of them pass `-`, so the gate uses a private mktemp file.
#   - the guard reads the command's OUTPUT, so a test whose own stdout printed a
#     libtest summary or per-test line could satisfy it without running. This is
#     pre-existing (the pre-fix counter had the same property) and no test in
#     this repository emits those lines. A lane that guards against a
#     test-tampering adversary needs a different instrument, not a stricter
#     pattern here.
#
# usage: exact-gate.sh <log-path|-> <command...>
set -uo pipefail

log="${1:-}"
shift 2>/dev/null
if [ -z "$log" ] || [ "$#" -eq 0 ]; then
    printf 'exact-gate: usage: exact-gate.sh <log-path|-> <command...>\n' >&2
    exit 2
fi

if [ "$log" = "-" ]; then
    cleanup=1
    log=$(mktemp)
else
    cleanup=0
    mkdir -p "$(dirname "$log")" 2>/dev/null
fi

names=$(printf '%s\n' "$*" | grep -o -- '--exact' | wc -l | tr -d ' ')
if [ "${names:-0}" -lt 1 ]; then
    printf 'exact-gate: REFUSED: the guarded command carries no --exact filter\n' >&2
    exit 2
fi

"$@" >"$log" 2>&1
status=$?
cat "$log"

executed=$(awk '
    # Per-test lines are libtest'"'"'s default output. They count one execution
    # per test, so a single invocation naming several --exact filters reports
    # several executions, not one.
    /^test [^[:space:]]+ \.\.\. (ok|FAILED)/ { from_tests++ }
    # Result lines are the fallback: a lane using --quiet or a terse format
    # prints no per-test lines, but always prints the summary. Summing
    # passed+failed over them counts executions without double counting, since
    # a summary line reports totals rather than one event.
    /^test result:/ {
        passed = 0
        failed = 0
        for (i = 1; i <= NF; i++) {
            if ($i == "passed;") passed = $(i - 1) + 0
            if ($i == "failed;") failed = $(i - 1) + 0
        }
        from_summary += passed + failed
    }
    END {
        count = from_tests > from_summary ? from_tests : from_summary
        print count + 0
    }
' "$log")
printf 'exact-gate: exit=%s named=%s executed=%s log=%s\n' "$status" "$names" "$executed" "$log"

if [ "$status" -eq 0 ] && [ "$executed" -ge "$names" ]; then
    if [ "$cleanup" -eq 1 ]; then
        rm -f "$log"
    fi
    exit 0
fi

if [ "$status" -ne 0 ]; then
    printf 'exact-gate: REFUSED: the guarded command exited %s; %s --exact filter(s) named, %s executed. The failure above is the command'\''s own, not a renamed or removed test (see the log for the failing assertion).\n' \
        "$status" "$names" "$executed" >&2
else
    printf 'exact-gate: REFUSED: %s --exact filter(s) named but %s executed; a renamed or removed test would have passed as an empty run\n' \
        "$names" "$executed" >&2
fi
exit 1
