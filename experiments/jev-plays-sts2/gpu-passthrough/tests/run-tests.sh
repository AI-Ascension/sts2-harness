#!/usr/bin/env bash
# Exercises 01-create-vf.sh and 99-rollback.sh against a mock host: a fake sysfs tree and a
# fake virsh whose domain states live in files. Nothing here touches a real host, so it is safe
# to run anywhere, including as an unprivileged user.
#
#   bash tests/run-tests.sh
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PREP="$(cd "$HERE/.." && pwd)"
pass=0; fail=0

report() {
    if [ "$1" = ok ]; then printf '  ok    %s\n' "$2"; pass=$((pass+1))
    else printf '  FAIL  %s\n     %s\n' "$2" "$3"; fail=$((fail+1)); fi
}

# A mock host: sysfs files that behave like the real ones (a write is readable back), and a
# virsh whose state is a file per domain.
make_host() {
    root="$(mktemp -d)"
    mkdir -p "$root/dev/0000:07:00.0" "$root/state" "$root/bin"
    echo "${1:-1}" > "$root/dev/0000:07:00.0/sriov_numvfs"
    echo "7" > "$root/dev/0000:07:00.0/sriov_totalvfs"
    mkdir -p "$root/dev/0000:07:00.1" "$root/dev/0000:07:00.2"
    ln -s "../0000:07:00.1" "$root/dev/0000:07:00.0/virtfn0"
    ln -s "../0000:07:00.2" "$root/dev/0000:07:00.0/virtfn1"
    echo "${2:-running}" > "$root/state/win"
    echo "${3:-running}" > "$root/state/lin"
    cat > "$root/bin/virsh" <<'MOCK'
#!/usr/bin/env bash
# Mock virsh. STATE_DIR holds one file per domain; REFUSE_SHUTDOWN makes a domain ignore it.
sd="$STATE_DIR"
f() { case "$1" in *windows*) echo "$sd/win" ;; *) echo "$sd/lin" ;; esac; }
case "${1:-}" in
    version)  exit 0 ;;
    domstate) cat "$(f "$2")" ;;
    shutdown) [ "${REFUSE_SHUTDOWN:-}" = "$2" ] || echo "shut off" > "$(f "$2")" ;;
    start)    echo running > "$(f "$2")"; echo "Domain '$2' started" ;;
    resume)   echo running > "$(f "$2")" ;;
    destroy)  echo "shut off" > "$(f "$2")" ;;
    detach-device) echo "Device detached successfully" ;;
    attach-device) echo "Device attached successfully" ;;
    *) exit 0 ;;
esac
MOCK
    chmod +x "$root/bin/virsh"
    echo "$root"
}

run_step() {
    local root="$1" script="$2"; shift 2
    STATE_DIR="$root/state" \
    JEV_GPU_DEVROOT="$root/dev" \
    JEV_VIRSH="$root/bin/virsh" \
    JEV_WAIT_TRIES=2 JEV_WAIT_SLEEP=0 \
    JEV_SETTLE_TRIES=2 JEV_SETTLE_SLEEP=0 JEV_START_SETTLE=0 \
    "$@" bash "$PREP/$script" 2>&1
}

printf '\n== 01-create-vf.sh\n'

root="$(make_host 1 running)"
out="$(run_step "$root" 01-create-vf.sh)"; rc=$?
numvfs="$(cat "$root/dev/0000:07:00.0/sriov_numvfs")"; win="$(cat "$root/state/win")"
[ "$rc" = 0 ] && [ "$numvfs" = 2 ] && [ "$win" = running ] \
    && report ok "happy path: reaches 2 VFs and restarts Windows" \
    || report fail "happy path: reaches 2 VFs and restarts Windows" "rc=$rc numvfs=$numvfs win=$win"
rm -rf "$root"

root="$(make_host 1 "shut off")"
out="$(run_step "$root" 01-create-vf.sh)"; rc=$?
win="$(cat "$root/state/win")"
[ "$rc" = 0 ] && [ "$win" = "shut off" ] \
    && report ok "a Windows domain that was off stays off" \
    || report fail "a Windows domain that was off stays off" "rc=$rc win=$win"
rm -rf "$root"

root="$(make_host 2 running)"
out="$(run_step "$root" 01-create-vf.sh)"; rc=$?
case "$out" in *"nothing to do"*) hit=yes ;; *) hit=no ;; esac
[ "$rc" = 0 ] && [ "$hit" = yes ] && [ "$(cat "$root/state/win")" = running ] \
    && report ok "already at 2 VFs is a no-op that never stops Windows" \
    || report fail "already at 2 VFs is a no-op that never stops Windows" "rc=$rc hit=$hit"
rm -rf "$root"

# The defect that shipped in the first draft: it stopped Windows, then discovered it could not write.
root="$(make_host 1 running)"
chmod 444 "$root/dev/0000:07:00.0/sriov_numvfs"
out="$(run_step "$root" 01-create-vf.sh)"; rc=$?
win="$(cat "$root/state/win")"
[ "$rc" != 0 ] && [ "$win" = running ] \
    && report ok "unwritable sysfs refuses before stopping Windows" \
    || report fail "unwritable sysfs refuses before stopping Windows" "rc=$rc win=$win"
chmod 644 "$root/dev/0000:07:00.0/sriov_numvfs"; rm -rf "$root"

root="$(make_host 1 running)"
out="$(REFUSE_SHUTDOWN=sts.home.complete.tech-windows run_step "$root" 01-create-vf.sh)"; rc=$?
win="$(cat "$root/state/win")"; numvfs="$(cat "$root/dev/0000:07:00.0/sriov_numvfs")"
[ "$rc" != 0 ] && [ "$win" = running ] && [ "$numvfs" = 1 ] \
    && report ok "a Windows domain that will not stop aborts without touching the card" \
    || report fail "a Windows domain that will not stop aborts without touching the card" "rc=$rc win=$win numvfs=$numvfs"
rm -rf "$root"

root="$(make_host 1 running)"
rm -rf "$root/dev/0000:07:00.2" "$root/dev/0000:07:00.0/virtfn1"
out="$(run_step "$root" 01-create-vf.sh)"; rc=$?
win="$(cat "$root/state/win")"
[ "$rc" != 0 ] && [ "$win" = running ] \
    && report ok "a missing VF1 fails but still restarts Windows" \
    || report fail "a missing VF1 fails but still restarts Windows" "rc=$rc win=$win"
rm -rf "$root"

printf '\n== 99-rollback.sh\n'

root="$(make_host 2 running running)"
out="$(run_step "$root" 99-rollback.sh)"; rc=$?
numvfs="$(cat "$root/dev/0000:07:00.0/sriov_numvfs")"
[ "$rc" = 0 ] && [ "$numvfs" = 1 ] && [ "$(cat "$root/state/win")" = running ] && [ "$(cat "$root/state/lin")" = running ] \
    && report ok "rollback returns to 1 VF and restarts both domains" \
    || report fail "rollback returns to 1 VF and restarts both domains" "rc=$rc numvfs=$numvfs"
rm -rf "$root"

root="$(make_host 2 "shut off" running)"
out="$(run_step "$root" 99-rollback.sh)"; rc=$?
[ "$rc" = 0 ] && [ "$(cat "$root/state/win")" = "shut off" ] && [ "$(cat "$root/state/lin")" = running ] \
    && report ok "rollback leaves an already-off domain off" \
    || report fail "rollback leaves an already-off domain off" "rc=$rc win=$(cat "$root/state/win")"
rm -rf "$root"

root="$(make_host 2 running running)"
out="$(REFUSE_SHUTDOWN=sts.home.complete.tech-slay-the-spire run_step "$root" 99-rollback.sh)"; rc=$?
numvfs="$(cat "$root/dev/0000:07:00.0/sriov_numvfs")"
[ "$rc" != 0 ] && [ "$numvfs" = 2 ] && [ "$(cat "$root/state/win")" = running ] && [ "$(cat "$root/state/lin")" = running ] \
    && report ok "a stuck domain aborts the rollback and both domains are put back" \
    || report fail "a stuck domain aborts the rollback and both domains are put back" "rc=$rc numvfs=$numvfs win=$(cat "$root/state/win") lin=$(cat "$root/state/lin")"
rm -rf "$root"

printf '\n== %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
