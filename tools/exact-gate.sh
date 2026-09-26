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
# The executed count is the number of per-test libtest result lines (`test <name>
# ... ok` or `... FAILED`), NOT a count of `test result: ok. 1 passed;` summaries.
# A summary line counts *passes*, so it reports 0 for a test that ran and failed,
# and for a command that names two `--exact` filters and therefore prints
# `test result: ok. 2 passed;` -- neither of which is an empty run. Counting
# executions keeps the empty-run protection (a filter matching nothing prints no
# per-test line) while making the count mean what the banner says it means
# (sts2-harness#540).
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
#   - a test that captures or forwards libtest's own output can print a line that
#     looks like a per-test result, which would count as an extra execution. No
#     lane does this, and the consequence is a stricter gate, not a weaker one.
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
    log=$(mktemp) || {
        printf 'exact-gate: REFUSED: mktemp could not create a log file (TMPDIR=%s)\n' "${TMPDIR:-<unset>}" >&2
        exit 2
    }
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

executed=$(awk '/^test [^[:space:]]+ \.\.\. (ok|FAILED)$/ {count++} END {print count+0}' "$log")
passed=$(awk '/^test [^[:space:]]+ \.\.\. ok$/ {count++} END {print count+0}' "$log")
printf 'exact-gate: exit=%s named=%s matched=%s passed=%s log=%s\n' \
    "$status" "$names" "$executed" "$passed" "$log"

if [ "$status" -eq 0 ] && [ "$executed" -ge "$names" ]; then
    if [ "$cleanup" -eq 1 ]; then
        rm -f "$log"
    fi
    exit 0
fi

if [ "$status" -ne 0 ]; then
    printf 'exact-gate: REFUSED: the guarded command exited %s after executing %s of %s named --exact filter(s); this is a command failure, not an empty run\n' \
        "$status" "$executed" "$names" >&2
else
    printf 'exact-gate: REFUSED: %s --exact filter(s) named but %s executed; a renamed or removed test would have passed as an empty run\n' \
        "$names" "$executed" >&2
fi
exit 1
