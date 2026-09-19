#!/usr/bin/env bash
# Undo everything: take the VF away from the Linux domain and put the card back to one VF.
#
# Each domain is returned to the state it was in when this script started, on every exit path,
# so a failure part way through does not leave the host with both domains switched off.
set -euo pipefail

PF="${JEV_GPU_PF:-0000:07:00.0}"
DEVROOT="${JEV_GPU_DEVROOT:-/sys/bus/pci/devices}"
LINUX="${JEV_LINUX_DOMAIN:-sts.home.complete.tech-slay-the-spire}"
WINDOWS="${JEV_WINDOWS_DOMAIN:-sts.home.complete.tech-windows}"
HERE="$(cd "$(dirname "$0")" && pwd)"
V="${JEV_VIRSH:-virsh -c qemu:///system}"
NUMVFS="$DEVROOT/$PF/sriov_numvfs"

# Poll timings. The defaults are what this host needs; the tests override them so the suite
# does not spend minutes sleeping through the abort paths.
WAIT_TRIES="${JEV_WAIT_TRIES:-90}"
WAIT_SLEEP="${JEV_WAIT_SLEEP:-2}"
SETTLE_TRIES="${JEV_SETTLE_TRIES:-15}"
SETTLE_SLEEP="${JEV_SETTLE_SLEEP:-1}"
START_SETTLE="${JEV_START_SETTLE:-5}"

say() { printf '\n== %s\n' "$1"; }
die() { echo "$1" >&2; exit 1; }

# Checked first: failing the sysfs write once both domains are down is the worst outcome here.
[ -w "$NUMVFS" ] || die "cannot write $NUMVFS - run this as root, with sudo"
$V version >/dev/null 2>&1 || die "cannot reach libvirt ($V)"

say "recording what is running now, so it can be put back"
declare -A was
for d in "$LINUX" "$WINDOWS"; do
    was[$d]="$($V domstate "$d" 2>/dev/null || echo unknown)"
    printf '%-45s %s\n' "$d" "${was[$d]}"
done

restore() {
    say "starting again whatever was running before"
    for d in "$WINDOWS" "$LINUX"; do
        case "${was[$d]}" in
            running|paused) $V start "$d" >/dev/null 2>&1 || true ;;
            *) echo "$d was ${was[$d]} before; leaving it alone" ;;
        esac
    done
    sleep "$START_SETTLE"
    for d in "$LINUX" "$WINDOWS"; do
        printf '%-45s %s\n' "$d" "$($V domstate "$d" 2>/dev/null || echo unknown)"
    done
}
# Any exit after this point puts the domains back, including the failure paths below.
trap restore EXIT

say "removing the hostdev from the Linux domain definition"
$V detach-device "$LINUX" "$HERE/hostdev-vf1.xml" --config || true

say "stopping both domains"
for d in "$LINUX" "$WINDOWS"; do
    state="${was[$d]}"
    if [ "$state" = paused ]; then
        $V resume "$d" || true
        state=running
    fi
    if [ "$state" = running ]; then
        $V shutdown "$d" || true
    fi
done
for _ in $(seq 1 "$WAIT_TRIES"); do
    a="$($V domstate "$LINUX" 2>/dev/null || echo unknown)"
    b="$($V domstate "$WINDOWS" 2>/dev/null || echo unknown)"
    [ "$a" = "shut off" ] && [ "$b" = "shut off" ] && break
    sleep "$WAIT_SLEEP"
done
for d in "$LINUX" "$WINDOWS"; do
    [ "$($V domstate "$d" 2>/dev/null)" = "shut off" ] \
        || die "$d did not shut down; not touching the card while a domain may hold a VF"
done

settle() {
    local want="$1"
    for _ in $(seq 1 "$SETTLE_TRIES"); do
        [ "$(cat "$NUMVFS")" -eq "$want" ] && return 0
        sleep "$SETTLE_SLEEP"
    done
    return 1
}

say "back to a single virtual function"
echo 0 > "$NUMVFS" || die "could not release the virtual functions"
settle 0 || die "sriov_numvfs did not reach 0"
echo 1 > "$NUMVFS" || die "could not ask for one virtual function"
settle 1 || die "sriov_numvfs did not reach 1"
cat "$NUMVFS"

say "the lane still has to be brought back by hand: 03-restart-lane.sh does the Steam and loop part"
