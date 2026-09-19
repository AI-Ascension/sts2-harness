#!/usr/bin/env bash
# Step 4. Confirm the game is rendering on the Arc and not on the CPU.
set -uo pipefail

LINUX=sts.home.complete.tech-slay-the-spire
G=/home/completetrain/guest_exec.sh

say() { printf '\n== %s\n' "$1"; }

say "vulkan devices the guest can see"
$G $LINUX /bin/sh -c 'sudo -u ubuntu env XDG_RUNTIME_DIR=/run/user/1000 timeout 25 vulkaninfo --summary 2>/dev/null | grep -iE "deviceName|driverName" | head -8'

say "what the game chose"
$G $LINUX /bin/sh -c 'f=$(ls -1t /home/ubuntu/sts2-native-map-g3-7d85w_n6/runtime-linux-rest-*-game-output/game.log | head -1); echo "$f"; grep -iE "Using Device|Rendering device name|FIFO protocol|VSync" "$f" | head -6'

say "cpu the game is burning (was 362% of one core on llvmpipe)"
$G $LINUX /bin/sh -c 'p=$(pgrep -f "[S]layTheSpire2" | head -1); [ -z "$p" ] && { echo "game not running"; exit 0; }; a=$(awk "{print \$14+\$15}" /proc/$p/stat); sleep 8; b=$(awk "{print \$14+\$15}" /proc/$p/stat); echo "$(( (b-a) * 100 / 8 / 100 ))% of one core over 8s"; nproc'

say "still fullscreen?"
$G $LINUX /bin/sh -c 'pid=$(pgrep -u ubuntu -f gnome-shell | head -1); dbus=$(tr "\0" "\n" < /proc/$pid/environ | grep "^DBUS_SESSION_BUS_ADDRESS=" | cut -d= -f2-); sudo -u ubuntu env DBUS_SESSION_BUS_ADDRESS="$dbus" gnome-extensions disable jev-fullscreen@complete.tech; sleep 2; sudo -u ubuntu env DBUS_SESSION_BUS_ADDRESS="$dbus" gnome-extensions enable jev-fullscreen@complete.tech; sleep 6; journalctl -b --no-pager -o cat --since "-1 min" | grep -F "jev-fullscreen enable:" | grep -v guest-exec | tail -1'

say "is the lane playing?"
$G $LINUX /bin/sh -c 'd=$(ls -1td /home/ubuntu/jev/runs/episode-* | head -1); echo "$d"; echo -n "exchanges: "; wc -l < "$d/jev-context.jsonl" 2>/dev/null || echo 0'
