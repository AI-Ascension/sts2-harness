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

# Extract EVERY committed `choose_revision` definition, from each
# `choose_revision() {` through the line that closes that function. Sourcing the
# real body is the point: every case below must be a test of the shipped
# workflow, not of a duplicate.
#
# bash resolves a function name to the LAST definition in the file, so a second
# copy silently replaces the guarded one. The extractor therefore must see all of
# them, and the suite must refuse to run its cases unless there is exactly one.
extract_functions() {
    awk '/^ *choose_revision\(\) \{$/ { inside = 1 }
         inside { print }
         inside && /^ *\}$/ { inside = 0 }' "$1" >"$2"
}

body="$tmp/choose_revision.body.sh"
extract_functions "$workflow" "$body"

# A definition only counts once its body terminates, so a truncated helper cannot
# read as a present one. `pending` is an opening brace that has not been closed
# yet, so the same counter answers both questions: how many definitions there
# are, and whether the last one was left dangling.
definitions=$(awk '/^ *choose_revision\(\) \{$/ { inside = 1; pending = 1; next }
               inside && /^ *\}$/ { inside = 0; pending = 0; n++ }
               END { print n + 0 }' "$body")

# Exactly one definition is the only state in which the cases below are a test of
# the helper the step actually calls. A duplicate is a hard bail rather than a
# failing case: continuing would test whichever copy the extractor happened to
# pick, which is the mechanism that hid this defect in the first place.
if [ "$definitions" -eq 0 ]; then
    printf 'Bail out! the runtime peer contract workflow no longer defines choose_revision()\n' >&2
    exit 2
elif [ "$definitions" -ne 1 ]; then
    printf 'Bail out! the runtime peer contract workflow defines choose_revision() %s times; bash uses the last definition, so the cases below would not test the helper the step actually calls\n' \
        "$definitions" >&2
    exit 2
fi

DEFAULT_GATEWAY_REVISION=3692f4d60fff06b8b4e1a44b14449dec6f30d67c
DEFAULT_MCP_REVISION=e6af39f30c14cd5ecc84be254511d4840bdb8fae

passed=0
failed=0

# line_is <output> <exact-line>
#
# Whole-line match on its own, so a value carrying a suffix cannot satisfy a
# prefix comparison. Padding the captured output with newlines lets the pattern
# anchor on both sides, which is what makes "this exact line" expressible.
line_is() {
    case $'\n'"$1"$'\n' in
        *$'\n'"$2"$'\n'*) return 0 ;;
        *) return 1 ;;
    esac
}

# The step's two call sites are extracted too, so `step_case` is a test of the
# shipped wiring rather than of a transcribed copy. If either assignment is
# renamed or removed the suite must say so instead of quietly asserting nothing.
for assignment in gateway_revision mcp_revision; do
    if ! grep -Eq "^[[:space:]]*$assignment=\\\$\(choose_revision " "$workflow"; then
        printf 'Bail out! the runtime peer contract workflow no longer assigns %s from choose_revision()\n' \
            "$assignment" >&2
        exit 2
    fi
done

# The driver reproduces the workflow step's own shape: source the extracted body,
# then call it in a command substitution. `MARKER_AFTER` stands for the step's
# continuation, so reaching it means the pin was not enforced -- a guard that
# returns non-zero but still yields a usable revision is not a pin.
#
# run_case <label> <supplied> <fallback> <expected-exit> <expect-captured:yes|no> [expected-value]
run_case() {
    label=$1
    supplied=$2
    fallback=$3
    expected_exit=$4
    expect_captured=$5
    # An accept case must pin WHICH value comes back, not merely that something
    # did. A success path that returned the fallback instead of the supplied
    # revision would leave the operator's pin ignored and the suite green, which
    # is the failure direction opposite to #561 and just as silent.
    expected_value=${6-}

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
    # Compared as a whole line, not as a substring: a value carrying a suffix
    # would satisfy a prefix match and still be the wrong revision.
    actual_value=no
    if line_is "$out" "CAPTURED=$expected_value"; then
        actual_value=yes
    fi
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
    if [ -n "$expected_value" ]; then
        [ "$actual_value" = yes ] || ok=no
    fi

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
        [ -n "$expected_value" ] &&
            printf '#   expected value=%s\n' "$expected_value"
        printf '#   actual   exit=%s captured=%s marker=%s diagnostic=%s\n' \
            "$status" "$actual_captured" "$actual_marker" "$actual_diagnostic"
        [ -n "$expected_value" ] &&
            printf '#   actual value=%s\n' \
                "$(printf '%s' "$out" | sed -n 's/^CAPTURED=//p' | tr '\n' ' ')"
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
# Each names the value it must come back with, so a helper that resolved every
# accepted input to the fallback would fail here rather than pass quietly.
run_case accept_gateway "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION" 0 yes "$DEFAULT_GATEWAY_REVISION"
run_case accept_mcp "$DEFAULT_MCP_REVISION" "$DEFAULT_MCP_REVISION" 0 yes "$DEFAULT_MCP_REVISION"
run_case accept_other_commit 0123456789abcdef0123456789abcdef01234567 "$DEFAULT_GATEWAY_REVISION" 0 yes 0123456789abcdef0123456789abcdef01234567

# The empty-input fallback is unchanged, so a plain `push` run with no dispatch
# input still resolves both peers.
run_case fallback_gateway "" "$DEFAULT_GATEWAY_REVISION" 0 yes "$DEFAULT_GATEWAY_REVISION"
run_case fallback_mcp "" "$DEFAULT_MCP_REVISION" 0 yes "$DEFAULT_MCP_REVISION"

# The step as the workflow writes it, both call sites, so a regression in the
# second resolution is caught and not just the helper in isolation. A success
# must additionally resolve to the EXPECTED pair, not merely exit 0.
#
# The two assignments are EXTRACTED from the workflow too. A transcribed driver
# would keep asserting the correct pairing while the shipped step wired the
# gateway input to the MCP fallback, and a suite that cannot see that would be
# green through a pin it never checked.
step_driver() {
    driver="$tmp/$1.driver.sh"
    {
        printf 'set -e\n'
        printf '. "$1"\n'
        # The extracted assignments name the workflow's own inputs and defaults,
        # so the driver defines those names and nothing else. That is what makes
        # the case a test of the shipped wiring: change which input reaches which
        # fallback in the workflow and the expectations below stop holding.
        printf 'INPUT_GATEWAY_REVISION=$2\n'
        printf 'INPUT_MCP_REVISION=$3\n'
        printf 'DEFAULT_GATEWAY_REVISION=%s\n' "$DEFAULT_GATEWAY_REVISION"
        printf 'DEFAULT_MCP_REVISION=%s\n' "$DEFAULT_MCP_REVISION"
        awk '/^[[:space:]]*gateway_revision=\$\(choose_revision/ { print; exit }' \
            "$workflow"
        awk '/^[[:space:]]*mcp_revision=\$\(choose_revision/ { print; exit }' \
            "$workflow"
        printf 'printf "gateway_revision=%%s\\n" "$gateway_revision"\n'
        printf 'printf "mcp_revision=%%s\\n" "$mcp_revision"\n'
    } >"$driver"
}

step_case() {
    label=$1
    gateway_input=$2
    mcp_input=$3
    expected_exit=$4
    expected_gateway=$5
    expected_mcp=$6

    step_driver "$label"
    out=$(bash -e "$driver" "$body" \
        "$gateway_input" "$mcp_input" 2>&1)
    status=$?

    # When the step is refused it must not have written either revision: the
    # `$GITHUB_OUTPUT` append for a bogus ref is the impact #561 describes.
    ok=yes
    [ "$status" = "$expected_exit" ] || ok=no
    if [ "$expected_exit" != 0 ]; then
        case $out in
            *gateway_revision=*|*mcp_revision=*) ok=no ;;
        esac
    else
        # The pair the step would write to $GITHUB_OUTPUT has to be the pair the
        # operator asked for. A step that exits 0 while resolving both inputs to
        # the defaults would otherwise look identical to a correctly pinned run.
        line_is "$out" "gateway_revision=$expected_gateway" || ok=no
        line_is "$out" "mcp_revision=$expected_mcp" || ok=no
    fi

    if [ "$ok" = yes ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        if [ "$expected_exit" = 0 ]; then
            printf '#   expected exit=0 resolving to gateway=%s mcp=%s\n' \
                "$expected_gateway" "$expected_mcp"
        else
            printf '#   expected exit=%s with no revision written\n' "$expected_exit"
        fi
        printf '#   actual   exit=%s output=%s\n' \
            "$status" "$(printf '%s' "$out" | tr '\n' ' ')"
    fi
}

step_case step_defaults "" "" 0 "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION"
step_case step_rejects_bad_gateway "refs/heads/attacker-branch" "" 1 "" ""
step_case step_rejects_bad_mcp "" "refs/heads/attacker-branch" 1 "" ""
step_case step_accepts_explicit_pair "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION" 0 \
    "$DEFAULT_GATEWAY_REVISION" "$DEFAULT_MCP_REVISION"

# A mixed pair: each call site resolves independently, so one supplied and one
# empty must produce the supplied revision for its peer and the default for the
# other. A helper that ignored its argument would collapse this to two defaults.
step_case step_mixed_pair "$DEFAULT_MCP_REVISION" "" 0 \
    "$DEFAULT_MCP_REVISION" "$DEFAULT_MCP_REVISION"

# The extracted body must be the shipped one. If someone refactors the helper out
# of the workflow this suite must fail loudly rather than silently test nothing,
# and a second definition must be counted rather than skipped -- the defect this
# suite exists to catch is reintroduced precisely by a duplicate the extractor
# cannot see.
definition_case() {
    label=$1
    text=$2
    expected=$3

    candidate="$tmp/$label.defs.sh"
    printf '%s\n' "$text" | awk '/^ *choose_revision\(\) \{$/ { inside = 1 }
         inside { print }
         inside && /^ *\}$/ { inside = 0 }' >"$candidate"

    # A definition only counts once its body terminates; a truncated helper must
    # not read as a present one.
    found=$(awk '/^ *choose_revision\(\) \{$/ { inside = 1; next }
                 inside && /^ *\}$/ { inside = 0; n++ }
                 END { print n + 0 }' "$candidate")
    if [ "$found" = "$expected" ]; then
        passed=$((passed + 1))
        printf 'ok %d - %s\n' "$((passed + failed))" "$label"
    else
        failed=$((failed + 1))
        printf 'not ok %d - %s\n' "$((passed + failed))" "$label"
        printf '#   expected %s complete choose_revision() definition(s), got %s\n' \
            "$expected" "$found"
    fi
}

definition_case real_workflow_defines_the_helper_once "$(cat "$workflow")" 1
definition_case missing_helper_is_counted_as_zero 'name: no helper here' 0

# The shadowing case, spelled out. Two definitions must read as two, because bash
# would run the second one and the cases above would still be testing the first.
shadowing='choose_revision() {
  printf "%s" "$1"
}
choose_revision() {
  printf "%s" "$2"
}'
definition_case shadowing_duplicate_is_counted_as_two "$shadowing" 2

unterminated='choose_revision() {
  printf "%s" "$1"'
definition_case unterminated_helper_is_not_a_definition "$unterminated" 0

printf 'runtime-peer-contract-revision-selftest: %d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
