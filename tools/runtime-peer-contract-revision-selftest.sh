#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Regression tests for `runtime-peer-contract.yml`'s `choose_revision` guard
# (sts2-harness#561).
#
# The workflow's `workflow_dispatch` inputs are declared as "Exact 40-hex ...
# candidate revision", and `choose_revision` guarded them with
# `printf '%s' "$supplied" | grep -Eq '^[0-9a-f]{40}$'`. That guard was inert: the
# function's last command was an unconditional `printf '%s' "$supplied"`, so
# `choose_revision` returned 0 for ANY supplied value, `refs/heads/attacker-branch`
# was written to `$GITHUB_OUTPUT`, and both peer checkouts followed an
# operator-supplied ref. The lane's later "Verify peer revisions" step cannot catch
# that, because it compares `rev-parse HEAD` against the same unvalidated value.
#
# TWO independent bash rules had to be defeated together, and both were measured
# against the unfixed body before the fix was written:
#
#   1. `errexit` does NOT propagate into a command-substitution subshell:
#      `bash -e -c 'g() { false; echo tail; }; r=$(g)'` exits 0 and captures `tail`.
#   2. A function's exit status is the status of its LAST command, so even where
#      `errexit` does apply, the trailing `printf` masks the failing `grep`.
#
# Either rule alone is enough, so every case below runs the function through a
# command substitution, exactly as the workflow writes it. A bare call is
# deliberately NOT used: under `bash -e` a bare function name in command position
# aborts even on the unfixed body (a special-builtin rule), so a bare-call suite
# would pass against the defect it is meant to catch.
#
# The function body is EXTRACTED from the committed workflow rather than copied
# here, so a regression in the workflow itself is what fails. A transcribed copy
# would let the workflow revert to the inert body while this suite stayed green.
#
# No cargo is required, so this runs on every PR in the `policy` gate beside the
# lane it protects.
# usage: runtime-peer-contract-revision-selftest.sh [path-to-workflow]
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
workflow=${1:-"$here/../.github/workflows/runtime-peer-contract.yml"}

if [ ! -f "$workflow" ]; then
    printf 'Bail out! cannot read the runtime peer contract workflow: %s\n' "$workflow" >&2
    exit 2
fi

tmp=$(mktemp -d) || exit 2
trap 'find "$tmp" -mindepth 1 -delete; rmdir "$tmp" 2>/dev/null' EXIT

# Extract the committed `choose_revision` definition, from `choose_revision() {`
# through the line that closes the function. Sourcing the real body is the point:
# every case below must be a test of the shipped workflow, not of a duplicate.
extract_function() {
    awk '/^ *choose_revision\(\) \{$/ { inside = 1 }
         inside { print }
         inside && /^ *\}$/ { exit }' "$1" >"$2"
}

body="$tmp/choose_revision.body.sh"
extract_function "$workflow" "$body"

if ! grep -Eq '^[[:space:]]*choose_revision\(\) \{$' "$body"; then
    printf 'Bail out! the runtime peer contract workflow no longer defines choose_revision()\n' >&2
    exit 2
fi
if ! tail -1 "$body" | grep -Eq '^[[:space:]]*\}$'; then
    printf 'Bail out! could not extract the end of choose_revision() from the workflow\n' >&2
    exit 2
fi

DEFAULT_GATEWAY_REVISION=3692f4d60fff06b8b4e1a44b14449dec6f30d67c
DEFAULT_MCP_REVISION=e6af39f30c14cd5ecc84be254511d4840bdb8fae

passed=0
failed=0

# The driver reproduces the workflow step's own shape: source the extracted body,
# then call it in a command substitution. `MARKER_AFTER` stands for the step's
# continuation, so reaching it means the pin was not enforced -- a guard that
# returns non-zero but still yields a usable revision is not a pin.
#
# run_case <label> <supplied> <fallback> <expected-exit> <expect-captured:yes|no>
run_case() {
    label=$1
    supplied=$2
    fallback=$3
    expected_exit=$4
    expect_captured=$5

    driver="$tmp/$label.driver.sh"
    cat >"$driver" <<'DRIVER'
set -e
. "$1"
result=$("${2}" "$3" "$4")
printf 'CAPTURED=%s\n' "$result"
printf 'MARKER_AFTER\n'
DRIVER

    out=$(bash -e "$driver" "$body" choose_revision "$supplied" "$fallback" 2>&1)
    status=$?

    actual_captured=no
    case $out in
        *CAPTURED=*) actual_captured=yes ;;
    esac
    actual_marker=no
    case $out in
        *MARKER_AFTER*) actual_marker=yes ;;
    esac
    # A refusal must also explain itself, otherwise a red lane gives the operator a
    # bare non-zero exit with no indication of which input was wrong.
    actual_diagnostic=no
    case $out in
        *"invalid immutable peer revision"*) actual_diagnostic=yes ;;
    esac

    ok=yes
    [ "$status" = "$expected_exit" ] || ok=no
    [ "$actual_captured" = "$expect_captured" ] || ok=no

    # Only a success may carry the step on; a refusal must produce no value to
    # carry on with, and must have said why.
    if [ "$expected_exit" = 0 ]; then
        [ "$actual_marker" = yes ] || ok=no
    else
        [ "$actual_marker" = no ] || ok=no
        [ "$actual_diagnostic" = yes ] || ok=no
    fi

    if [ "$ok" = yes ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        printf '#   expected exit=%s captured=%s\n' "$expected_exit" "$expect_captured"
        printf '#   actual   exit=%s captured=%s marker=%s diagnostic=%s\n' \
            "$status" "$actual_captured" "$actual_marker" "$actual_diagnostic"
        printf '%s\n' "$out" | sed 's/^/#   /'
    fi
}

# The reported defect, as a regression test: a non-40-hex ref is refused and the
# step stops. Against the unfixed body every one of these exits 0, captures the
# attacker's ref, and reaches MARKER_AFTER.
run_case reject_branch refs/heads/attacker-branch "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_tag refs/tags/v1.2.3 "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_pull_ref refs/pull/1/head "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_bare_branch main "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_short_sha e6af39f3 "$DEFAULT_MCP_REVISION" 1 no
run_case reject_garbage "not a revision" "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_39_hex 3692f4d60fff06b8b4e1a44b14449dec6f30d67 "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_41_hex 3692f4d60fff06b8b4e1a44b14449dec6f30d67c0 "$DEFAULT_GATEWAY_REVISION" 1 no
# The guard's pattern is lower-case only, so an upper-case SHA-1 is not 40-hex as
# this workflow defines it and must be refused rather than quietly normalised.
run_case reject_upper_case 3692F4D60FFF06B8B4E1A44B14449DEC6F30D67C "$DEFAULT_GATEWAY_REVISION" 1 no
run_case reject_traversal "../../etc" "$DEFAULT_GATEWAY_REVISION" 1 no

# A valid 40-hex commit is returned unchanged and the step carries on.
run_case accept_gateway "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION" 0 yes
run_case accept_mcp "$DEFAULT_MCP_REVISION" "$DEFAULT_MCP_REVISION" 0 yes
run_case accept_other_commit 0123456789abcdef0123456789abcdef01234567 "$DEFAULT_GATEWAY_REVISION" 0 yes

# The empty-input fallback is unchanged, so a plain `push` run with no dispatch
# input still resolves both peers.
run_case fallback_gateway "" "$DEFAULT_GATEWAY_REVISION" 0 yes
run_case fallback_mcp "" "$DEFAULT_MCP_REVISION" 0 yes

# The step as the workflow writes it, both call sites, so a regression in the
# second resolution is caught and not just the helper in isolation.
step_case() {
    label=$1
    gateway_input=$2
    mcp_input=$3
    expected_exit=$4

    driver="$tmp/$label.driver.sh"
    cat >"$driver" <<'DRIVER'
set -e
. "$1"
gateway_revision=$("${2}" "$3" "$4")
mcp_revision=$("${2}" "$5" "$6")
printf 'gateway_revision=%s\n' "$gateway_revision"
printf 'mcp_revision=%s\n' "$mcp_revision"
DRIVER

    out=$(bash -e "$driver" "$body" choose_revision \
        "$gateway_input" "$DEFAULT_GATEWAY_REVISION" \
        "$mcp_input" "$DEFAULT_MCP_REVISION" 2>&1)
    status=$?

    # When the step is refused it must not have written either revision: the
    # `$GITHUB_OUTPUT` append for a bogus ref is the impact #561 describes.
    ok=yes
    [ "$status" = "$expected_exit" ] || ok=no
    if [ "$expected_exit" != 0 ]; then
        case $out in
            *gateway_revision=*|*mcp_revision=*) ok=no ;;
        esac
    fi

    if [ "$ok" = yes ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        printf '#   expected exit=%s with no revision written\n' "$expected_exit"
        printf '#   actual   exit=%s output=%s\n' \
            "$status" "$(printf '%s' "$out" | tr '\n' ' ')"
    fi
}

step_case step_defaults "" "" 0
step_case step_rejects_bad_gateway "refs/heads/attacker-branch" "" 1
step_case step_rejects_bad_mcp "" "refs/heads/attacker-branch" 1
step_case step_accepts_explicit_pair "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION" 0

# The extracted body must be the shipped one. If someone refactors the helper out
# of the workflow this suite must fail loudly rather than silently test nothing.
extraction_case() {
    label=$1
    text=$2
    expected=$3

    candidate="$tmp/$label.body.sh"
    printf '%s\n' "$text" | awk '/^ *choose_revision\(\) \{$/ { inside = 1 }
         inside { print }
         inside && /^ *\}$/ { exit }' >"$candidate"

    # Mirror the real validation: the opening brace must be found AND the body
    # must terminate, otherwise a truncated helper would look like a present one.
    found=absent
    if grep -Eq '^[[:space:]]*choose_revision\(\) \{$' "$candidate" &&
        tail -1 "$candidate" | grep -Eq '^[[:space:]]*\}$'; then
        found=present
    fi
    if [ "$found" = "$expected" ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        printf '#   expected the extractor to report %s, got %s\n' "$expected" "$found"
    fi
}

extraction_case extract_real_workflow "$(cat "$workflow")" present
extraction_case extract_missing_function 'name: no helper here' absent

unterminated='choose_revision() {
  printf "%s" "$1"'
extraction_case extract_unterminated "$unterminated" absent

printf 'runtime-peer-contract-revision-selftest: %d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
