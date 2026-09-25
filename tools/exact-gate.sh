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
# as `<name> -- --ignored --exact --nocapture`. The matched count is the number of
# `test result: ok. 1 passed;` lines, which is what a single-test filter emits.
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

matched=$(awk '/^test result: ok\. 1 passed;/ {count++} END {print count+0}' "$log")
printf 'exact-gate: exit=%s named=%s matched=%s log=%s\n' "$status" "$names" "$matched" "$log"

if [ "$status" -eq 0 ] && [ "$matched" -ge "$names" ]; then
    if [ "$cleanup" -eq 1 ]; then
        rm -f "$log"
    fi
    exit 0
fi

printf 'exact-gate: REFUSED: %s --exact filter(s) named but %s executed; a renamed or removed test would have passed as an empty run\n' \
    "$names" "$matched" >&2
exit 1
