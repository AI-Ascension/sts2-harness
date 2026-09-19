#!/usr/bin/env bash
# Undo everything: take the VF away from the Linux domain and put the card back to one VF.
#
# Each domain is returned to the state it was in when this script started, so running it on a
# host where one domain was deliberately shut off does not quietly turn that domain on.
set -euo pipefail

PF=0000:07:00.0
LINUX=sts.home.complete.tech-slay-the-spire
WINDOWS=sts.home.complete.tech-windows
HERE="$(cd "$(dirname "$0")" && pwd)"
V="virsh -c qemu:///system"
NUMVFS=/sys/bus/pci/devices/$PF/sriov_numvfs

say() { printf '\n== %s\n' "$1"; }
die() { echo "$1" >&2; exit 1; }

# Checked first: failing the sysfs write after both domains are down is the worst outcome here.
[ "$(id -u)" -eq 0 ] || die "must run as root (writes $NUMVFS); re-run with sudo"
[ -w "$NUMVFS" ] || die "$NUMVFS is not writable"
$V version >/dev/null 2>&1 || die "cannot reach qemu:///system"

say "recording what is running now, so it can be put back"
declare -A was
for d in "$LINUX" "$WINDOWS"; do
    was[$d]="$($V domstate "$d" 2>/dev/null || echo unknown)"
    printf '%-45s %s\n' "$d" "${was[$d]}"
done

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
for _ in $(seq 1 90); do
    a="$($V domstate "$LINUX" 2>/dev/null || echo unknown)"
    b="$($V domstate "$WINDOWS" 2>/dev/null || echo unknown)"
    [ "$a" = "shut off" ] && [ "$b" = "shut off" ] && break
    sleep 2
done
for d in "$LINUX" "$WINDOWS"; do
    [ "$($V domstate "$d" 2>/dev/null)" = "shut off" ] || die \
        "$d did not shut down; not touching the card while a domain may hold a VF"
done

settle() {
    local want="$1"
    for _ in $(seq 1 15); do
        [ "$(cat "$NUMVFS")" -eq "$want" ] && return 0
        sleep 1
    done
    return 1
}

say "back to a single virtual function"
echo 0 > "$NUMVFS"
settle 0 || die "sriov_numvfs did not reach 0"
echo 1 > "$NUMVFS"
settle 1 || die "sriov_numvfs did not reach 1"
cat "$NUMVFS"

say "starting again whatever was running before"
for d in "$WINDOWS" "$LINUX"; do
    case "${was[$d]}" in
        running|paused) $V start "$d" || true ;;
        *) echo "$d was $(printf '%s' "${was[$d]}") before; leaving it alone" ;;
    esac
done
sleep 5
for d in "$LINUX" "$WINDOWS"; do printf '%-45s %s\n' "$d" "$($V domstate "$d" 2>/dev/null)"; done

say "the lane still has to be brought back by hand: 03-restart-lane.sh does the Steam and loop part"
