#!/usr/bin/env bash
# Read-only. Changes nothing, starts nothing, stops nothing.
# Run this first, and again any time before step 1, to confirm the plan still holds.
set -uo pipefail

PF="${JEV_GPU_PF:-0000:07:00.0}"
VF0="${JEV_GPU_VF0:-0000:07:00.1}"
DEVROOT="${JEV_GPU_DEVROOT:-/sys/bus/pci/devices}"
LINUX="${JEV_LINUX_DOMAIN:-sts.home.complete.tech-slay-the-spire}"
WINDOWS="${JEV_WINDOWS_DOMAIN:-sts.home.complete.tech-windows}"
G="${JEV_GUEST_EXEC:-/home/completetrain/guest_exec.sh}"
V="${JEV_VIRSH:-virsh -c qemu:///system}"

pass=0
fail=0

# if/else rather than `test && ok || bad`: in that form the failure branch also runs when the
# success branch itself returns non-zero, which is exactly the kind of quiet wrong answer a
# preflight must not give.
check() {
    if [ "$1" = ok ]; then
        printf '  ok    %s\n' "$2"
        pass=$((pass + 1))
    else
        printf '  FAIL  %s -- %s\n' "$2" "$1"
        fail=$((fail + 1))
    fi
}

printf '\n== the card\n'
total="$(cat "$DEVROOT/$PF/sriov_totalvfs" 2>/dev/null)"
num="$(cat "$DEVROOT/$PF/sriov_numvfs" 2>/dev/null)"
echo "  totalvfs=${total:-?} numvfs=${num:-?}"
if [ "${total:-0}" -ge 2 ] 2>/dev/null; then
    check ok "physical function can provide 2 VFs"
else
    check "totalvfs=${total:-unreadable}" "physical function can provide 2 VFs"
fi
if [ -e "$DEVROOT/$VF0" ]; then
    check ok "VF0 present at $VF0"
else
    check "missing" "VF0 present at $VF0"
fi

printf '\n== libvirt\n'
if $V version >/dev/null 2>&1; then
    check ok "libvirt reachable"
else
    check "cannot connect" "libvirt reachable"
fi
for d in "$LINUX" "$WINDOWS"; do
    printf '  %-45s %s\n' "$d" "$($V domstate "$d" 2>/dev/null || echo unknown)"
done
n="$($V dumpxml --inactive "$LINUX" 2>/dev/null | grep -c '<hostdev')"
if [ "$n" = 0 ]; then
    check ok "Linux domain has no hostdev yet"
else
    check "found $n" "Linux domain has no hostdev yet"
fi
if $V dumpxml "$WINDOWS" 2>/dev/null | grep -q "function='0x1'"; then
    check ok "Windows hostdev still names function 0x1"
else
    check "changed - 01-create-vf.sh assumes this address" "Windows hostdev still names function 0x1"
fi

printf '\n== the guest can drive a VF\n'
mods="$("$G" "$LINUX" /bin/sh -c 'ls /lib/modules/$(uname -r)/kernel/drivers/gpu/drm/ 2>/dev/null | grep -cE "^xe$"' 2>/dev/null | tr -d '[:space:]')"
if [ "${mods:-0}" -ge 1 ] 2>/dev/null; then
    check ok "guest has the xe driver"
else
    check "not found" "guest has the xe driver"
fi
icd="$("$G" "$LINUX" /bin/sh -c 'ls /usr/share/vulkan/icd.d/intel_icd.json >/dev/null 2>&1 && echo yes || echo no' 2>/dev/null | tr -d '[:space:]')"
if [ "$icd" = yes ]; then
    check ok "guest has the Intel Vulkan ICD"
else
    check "${icd:-no answer}" "guest has the Intel Vulkan ICD"
fi

printf '\n== what the game is doing today (the thing being fixed)\n'
"$G" "$LINUX" /bin/sh -c 'f=$(ls -1t /home/ubuntu/sts2-native-map-g3-7d85w_n6/runtime-linux-rest-*-game-output/game.log 2>/dev/null | head -1); [ -n "$f" ] && grep -m1 -iE "Using Device" "$f"' 2>/dev/null

printf '\n== %d checks passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
