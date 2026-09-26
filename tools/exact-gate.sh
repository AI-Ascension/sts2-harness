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
# as `<name> -- --ignored --exact --nocapture`.
#
# The executed count is read from the per-test result lines, not from the
# `test result:` summary, because the summary cannot be told apart from an empty
# run by the pass count alone: a run that executed one test and that test FAILED
# prints `test result: FAILED. 0 passed; 1 failed;`, whose `0 passed` is
# indistinguishable from the `0 passed; N filtered out` of a run that executed
# nothing. Counting the `ok. 1 passed;` summary line therefore reported a test
# that ran and failed as `0 executed` and then told the reader it "would have
# passed as an empty run" -- an inverted account of the one situation where the
# message is read (sts2-harness#540).
#
# The pattern counts a test that executed and finished either way: libtest prints
# `test <name> ... ok` and `test <name> ... FAILED`, and prints `test <name> ...
# ignored` for a filtered-out test it did not execute. `ignored` is therefore
# deliberately not in the alternation, and a name is required to have no
# whitespace so the `- should panic` annotation cannot hide an execution.
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
#     distinguished from a renamed one, and is rejected rather than accepted;
#   - the executed count is read from the per-test lines, so a test that libtest
#     reports in another shape (a timeout, a panic outside a result line) is not
#     counted as executed and the gate still refuses rather than passing.
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

executed=$(awk '/^test [^[:space:]]+ \.\.\. (ok|FAILED)/ {count++} END {print count+0}' "$log")
printf 'exact-gate: exit=%s named=%s executed=%s log=%s\n' "$status" "$names" "$executed" "$log"

if [ "$status" -eq 0 ] && [ "$executed" -ge "$names" ]; then
    if [ "$cleanup" -eq 1 ]; then
        rm -f "$log"
    fi
    exit 0
fi

if [ "$executed" -ge "$names" ]; then
    printf 'exact-gate: REFUSED: %s --exact filter(s) named and %s executed, but the command exited %s; this is a test failure, not a missing test\n' \
        "$names" "$executed" "$status" >&2
    exit 1
fi

printf 'exact-gate: REFUSED: %s --exact filter(s) named but %s executed; a renamed or removed test would have passed as an empty run\n' \
    "$names" "$executed" >&2
exit 1
