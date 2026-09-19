#!/usr/bin/env bash
# Step 1. Give the physical function a second virtual function.
#
# The kernel will not change sriov_numvfs while a VF is assigned, and will not go from 1 to 2
# without passing through 0. VF0 belongs to the Windows domain, so that domain has to be down
# for the duration. It is started again at the end only if it was running to begin with.
set -euo pipefail

PF=0000:07:00.0
WINDOWS=sts.home.complete.tech-windows
WANT_VFS=2
V="virsh -c qemu:///system"
NUMVFS=/sys/bus/pci/devices/$PF/sriov_numvfs

say() { printf '\n== %s\n' "$1"; }
die() { echo "$1" >&2; exit 1; }

# Checked before anything is stopped. Failing the sysfs write later would leave the Windows
# domain shut down for nothing.
[ "$(id -u)" -eq 0 ] || die "must run as root (writes $NUMVFS); re-run with sudo"
[ -w "$NUMVFS" ] || die "$NUMVFS is not writable"
$V version >/dev/null 2>&1 || die "cannot reach qemu:///system"

say "before"
echo "numvfs   : $(cat "$NUMVFS")"
echo "totalvfs : $(cat /sys/bus/pci/devices/$PF/sriov_totalvfs)"
ls -l /sys/bus/pci/devices/$PF/ | grep virtfn || true

[ "$(cat /sys/bus/pci/devices/$PF/sriov_totalvfs)" -ge "$WANT_VFS" ] \
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
    for _ in $(seq 1 60); do
        [ "$($V domstate "$WINDOWS" 2>/dev/null)" = "shut off" ] && break
        sleep 2
    done
fi
[ "$($V domstate "$WINDOWS" 2>/dev/null)" = "shut off" ] || die \
    "the Windows domain did not shut down; stopping rather than forcing it.
Run 'virsh -c qemu:///system destroy $WINDOWS' and re-run, if that is acceptable."

# Each write is verified rather than slept at, because the driver tears the functions down
# asynchronously and a fixed sleep either wastes time or races.
settle() {
    local want="$1"
    for _ in $(seq 1 15); do
        [ "$(cat "$NUMVFS")" -eq "$want" ] && return 0
        sleep 1
    done
    return 1
}

say "reprovisioning the virtual functions"
if ! echo 0 > "$NUMVFS" 2>/dev/null; then
    echo "could not release the existing virtual functions - something still holds one" >&2
    echo "check:  lsof /dev/vfio/* ; virsh -c qemu:///system list --all" >&2
    [ "$windows_was_running" = yes ] && { echo "restarting the Windows domain" >&2; $V start "$WINDOWS" || true; }
    exit 1
fi
settle 0 || die "sriov_numvfs did not reach 0"
echo "$WANT_VFS" > "$NUMVFS"
settle "$WANT_VFS" || die "sriov_numvfs did not reach $WANT_VFS"

say "after"
cat "$NUMVFS"
for link in /sys/bus/pci/devices/$PF/virtfn*; do
    printf '%s -> %s\n' "$(basename "$link")" "$(basename "$(readlink -f "$link")")"
done

# The Windows domain's <hostdev> names 0000:07:00.1 literally, so it has to come back there.
[ -e /sys/bus/pci/devices/0000:07:00.1 ] \
    || die "VF0 did not come back at 0000:07:00.1 - the Windows hostdev address is now stale"
[ -e /sys/bus/pci/devices/0000:07:00.2 ] \
    || die "VF1 is not at 0000:07:00.2 - update hostdev-vf1.xml to the address printed above"

if [ "$windows_was_running" = yes ]; then
    say "starting the Windows domain again (it was running before)"
    $V start "$WINDOWS"
    sleep 5
    $V domstate "$WINDOWS"
else
    say "leaving the Windows domain shut off (it was not running before)"
fi

say "done - VF1 is 0000:07:00.2 and is free for the Linux domain"
