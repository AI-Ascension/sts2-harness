#!/usr/bin/env bash
# Step 1. Give the physical function a second virtual function.
#
# The kernel will not change sriov_numvfs while a VF is assigned, and will not go from 1 to 2
# without passing through 0. VF0 belongs to the Windows domain, so that domain has to be down
# for the duration. It is started again at the end only if it was running to begin with.
set -euo pipefail

PF="${JEV_GPU_PF:-0000:07:00.0}"
VF0="${JEV_GPU_VF0:-0000:07:00.1}"
VF1="${JEV_GPU_VF1:-0000:07:00.2}"
DEVROOT="${JEV_GPU_DEVROOT:-/sys/bus/pci/devices}"
WINDOWS="${JEV_WINDOWS_DOMAIN:-sts.home.complete.tech-windows}"
WANT_VFS="${JEV_GPU_WANT_VFS:-2}"
V="${JEV_VIRSH:-virsh -c qemu:///system}"
NUMVFS="$DEVROOT/$PF/sriov_numvfs"

# Poll timings. The defaults are what this host needs; the tests override them so the suite
# does not spend minutes sleeping through the abort paths.
WAIT_TRIES="${JEV_WAIT_TRIES:-60}"
WAIT_SLEEP="${JEV_WAIT_SLEEP:-2}"
SETTLE_TRIES="${JEV_SETTLE_TRIES:-15}"
SETTLE_SLEEP="${JEV_SETTLE_SLEEP:-1}"
START_SETTLE="${JEV_START_SETTLE:-5}"

say() { printf '\n== %s\n' "$1"; }
die() { echo "$1" >&2; exit 1; }

# Checked before anything is stopped. The first draft of this script shut the Windows domain
# down and only then discovered it could not write to sysfs, which leaves the host worse off
# than when it started. Writability is the real requirement, so test that rather than the uid.
[ -w "$NUMVFS" ] || die "cannot write $NUMVFS - run this as root, with sudo"
$V version >/dev/null 2>&1 || die "cannot reach libvirt ($V)"

say "before"
echo "numvfs   : $(cat "$NUMVFS")"
echo "totalvfs : $(cat "$DEVROOT/$PF/sriov_totalvfs")"
list_vfs() {
    local link found=no
    for link in "$DEVROOT/$PF"/virtfn*; do
        [ -e "$link" ] || continue
        printf '%s -> %s
' "$(basename "$link")" "$(basename "$(readlink -f "$link")")"
        found=yes
    done
    [ "$found" = yes ] || echo "(no virtual functions)"
}

list_vfs

[ "$(cat "$DEVROOT/$PF/sriov_totalvfs")" -ge "$WANT_VFS" ] \
    || die "the physical function cannot provide $WANT_VFS virtual functions"

if [ "$(cat "$NUMVFS")" -eq "$WANT_VFS" ]; then
    say "already at $WANT_VFS virtual functions - nothing to do"
    exit 0
fi

say "shutting the Windows domain down (it holds VF0)"
windows_was_running=no
state="$($V domstate "$WINDOWS" 2>/dev/null || echo unknown)"
echo "state: $state"
if [ "$state" = running ] || [ "$state" = paused ]; then
    windows_was_running=yes
    [ "$state" = paused ] && $V resume "$WINDOWS"
    $V shutdown "$WINDOWS"
    for _ in $(seq 1 "$WAIT_TRIES"); do
        [ "$($V domstate "$WINDOWS" 2>/dev/null)" = "shut off" ] && break
        sleep "$WAIT_SLEEP"
    done
fi
[ "$($V domstate "$WINDOWS" 2>/dev/null)" = "shut off" ] || die \
    "the Windows domain did not shut down; stopping rather than forcing it.
Run '$V destroy $WINDOWS' and re-run, if that is acceptable."

# Each write is verified rather than slept at, because the driver tears the functions down
# asynchronously and a fixed sleep either wastes time or races it.
settle() {
    local want="$1"
    for _ in $(seq 1 "$SETTLE_TRIES"); do
        [ "$(cat "$NUMVFS")" -eq "$want" ] && return 0
        sleep "$SETTLE_SLEEP"
    done
    return 1
}

restart_windows_if_it_was_running() {
    [ "$windows_was_running" = yes ] || return 0
    echo "restarting the Windows domain" >&2
    $V start "$WINDOWS" || true
}

say "reprovisioning the virtual functions"
if ! echo 0 > "$NUMVFS" 2>/dev/null; then
    echo "could not release the existing virtual functions - something still holds one" >&2
    echo "check:  lsof /dev/vfio/* ; $V list --all" >&2
    restart_windows_if_it_was_running
    exit 1
fi
if ! settle 0; then
    restart_windows_if_it_was_running
    die "sriov_numvfs did not reach 0"
fi
if ! echo "$WANT_VFS" > "$NUMVFS" 2>/dev/null; then
    restart_windows_if_it_was_running
    die "could not ask for $WANT_VFS virtual functions"
fi
if ! settle "$WANT_VFS"; then
    restart_windows_if_it_was_running
    die "sriov_numvfs did not reach $WANT_VFS"
fi

say "after"
cat "$NUMVFS"
list_vfs

# The Windows domain's <hostdev> names VF0's address literally, so it has to come back there.
if [ ! -e "$DEVROOT/$VF0" ]; then
    restart_windows_if_it_was_running
    die "VF0 did not come back at $VF0 - the Windows hostdev address is now stale"
fi
if [ ! -e "$DEVROOT/$VF1" ]; then
    restart_windows_if_it_was_running
    die "VF1 is not at $VF1 - update hostdev-vf1.xml to the address printed above"
fi

if [ "$windows_was_running" = yes ]; then
    say "starting the Windows domain again (it was running before)"
    $V start "$WINDOWS"
    sleep "$START_SETTLE"
    $V domstate "$WINDOWS"
else
    say "leaving the Windows domain shut off (it was not running before)"
fi

say "done - VF1 is $VF1 and is free for the Linux domain"
