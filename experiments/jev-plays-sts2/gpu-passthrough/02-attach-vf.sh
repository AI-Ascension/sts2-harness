#!/usr/bin/env bash
# Step 2. Add the virtual function to the Linux domain's persistent definition.
#
# --config only, so the running domain is untouched: the device appears the next time the
# domain is started. Do not run this before 01-create-vf.sh has created 0000:07:00.2, or the
# domain will refuse to start with a missing host device.
set -euo pipefail

LINUX="${JEV_LINUX_DOMAIN:-sts.home.complete.tech-slay-the-spire}"
HERE="$(cd "$(dirname "$0")" && pwd)"
V="${JEV_VIRSH:-virsh -c qemu:///system}"

if [ ! -e ${JEV_GPU_DEVROOT:-/sys/bus/pci/devices}/0000:07:00.2 ]; then
    echo "0000:07:00.2 does not exist yet; run 01-create-vf.sh first" >&2
    exit 1
fi

echo "== current hostdevs (expect none)"
$V dumpxml --inactive "$LINUX" | sed -n '/<hostdev/,/<\/hostdev>/p'

echo "== attaching"
$V attach-device "$LINUX" "$HERE/hostdev-vf1.xml" --config

echo "== persistent definition now has"
$V dumpxml --inactive "$LINUX" | sed -n '/<hostdev/,/<\/hostdev>/p'
echo
echo "The running domain is unchanged. 03-restart-lane.sh picks it up."
