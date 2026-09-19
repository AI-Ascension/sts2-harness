#!/usr/bin/env bash
# Read-only. Changes nothing, starts nothing, stops nothing.
# Run this first, and again any time before step 1, to confirm the plan still holds.
set -uo pipefail

PF=0000:07:00.0
LINUX=sts.home.complete.tech-slay-the-spire
WINDOWS=sts.home.complete.tech-windows
G=/home/completetrain/guest_exec.sh
V="virsh -c qemu:///system"

pass=0; fail=0
check() {
    if [ "$2" = ok ]; then printf '  ok    %s\n' "$1"; pass=$((pass+1))
    else printf '  FAIL  %s -- %s\n' "$1" "$2"; fail=$((fail+1)); fi
}

printf '\n== the card\n'
total="$(cat /sys/bus/pci/devices/$PF/sriov_totalvfs 2>/dev/null)"
num="$(cat /sys/bus/pci/devices/$PF/sriov_numvfs 2>/dev/null)"
echo "  totalvfs=$total numvfs=$num"
[ "${total:-0}" -ge 2 ] && check "physical function can provide 2 VFs" ok \
    || check "physical function can provide 2 VFs" "totalvfs=$total"
[ -e /sys/bus/pci/devices/0000:07:00.1 ] && check "VF0 present at 0000:07:00.1" ok \
    || check "VF0 present at 0000:07:00.1" "missing"

printf '\n== libvirt\n'
$V version >/dev/null 2>&1 && check "qemu:///system reachable" ok || check "qemu:///system reachable" "no"
for d in "$LINUX" "$WINDOWS"; do printf '  %-45s %s\n' "$d" "$($V domstate "$d" 2>/dev/null)"; done
n="$($V dumpxml --inactive "$LINUX" 2>/dev/null | grep -c hostdev)"
[ "$n" = 0 ] && check "Linux domain has no hostdev yet" ok || check "Linux domain has no hostdev yet" "found $n"
$V dumpxml "$WINDOWS" 2>/dev/null | grep -q "function='0x1'" \
    && check "Windows hostdev still names function 0x1" ok \
    || check "Windows hostdev still names function 0x1" "changed - update 01-create-vf.sh"

printf '\n== the guest can drive a VF\n'
mods="$($G $LINUX /bin/sh -c 'ls /lib/modules/$(uname -r)/kernel/drivers/gpu/drm/ 2>/dev/null | grep -cE "^xe$"' 2>/dev/null | tr -d "[:space:]")"
[ "${mods:-0}" -ge 1 ] && check "guest has the xe driver" ok || check "guest has the xe driver" "not found"
icd="$($G $LINUX /bin/sh -c 'ls /usr/share/vulkan/icd.d/intel_icd.json >/dev/null 2>&1 && echo yes || echo no' 2>/dev/null | tr -d "[:space:]")"
[ "$icd" = yes ] && check "guest has the Intel Vulkan ICD" ok || check "guest has the Intel Vulkan ICD" "$icd"

printf '\n== what the game is doing today (the thing being fixed)\n'
$G $LINUX /bin/sh -c 'f=$(ls -1t /home/ubuntu/sts2-native-map-g3-7d85w_n6/runtime-linux-rest-*-game-output/game.log 2>/dev/null | head -1); [ -n "$f" ] && grep -m1 -iE "Using Device" "$f"' 2>/dev/null

printf '\n== %d checks passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
