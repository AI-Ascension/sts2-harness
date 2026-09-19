#!/usr/bin/env bash
# Step 3. Restart the Linux domain so it picks up the virtual function, then bring the lane back.
#
# This is the only step that stops the game. Everything the lane needs after a cold start is
# here, because none of it is automatic: the desktop session, then Steam, then the loop.
set -uo pipefail

LINUX=sts.home.complete.tech-slay-the-spire
G=/home/completetrain/guest_exec.sh
V="virsh -c qemu:///system"

say() { printf '\n== %s\n' "$1"; }

say "stopping the loop and the game inside the guest"
$G $LINUX /bin/sh -c 'pkill -f "[j]ev-loop.sh"; sleep 2; pkill -f "[S]layTheSpire2"; sleep 3; echo stopped'

say "shutting the domain down"
$V shutdown "$LINUX" --mode agent 2>/dev/null || $V shutdown "$LINUX"
for _ in $(seq 1 90); do
    [ "$($V domstate "$LINUX" 2>/dev/null)" = "shut off" ] && break
    sleep 2
done
$V domstate "$LINUX"

say "starting it again"
$V start "$LINUX"
for _ in $(seq 1 90); do
    timeout 10 $V qemu-agent-command "$LINUX" '{"execute":"guest-ping"}' >/dev/null 2>&1 && break
    sleep 3
done
echo "guest agent answered"

say "waiting for the desktop session"
for _ in $(seq 1 40); do
    out="$($G $LINUX /bin/sh -c 'pgrep -u ubuntu -f gnome-shell | head -1' 2>/dev/null)"
    case "$out" in ''|*failed*) sleep 5 ;; *) echo "gnome-shell pid $out"; break ;; esac
done

say "did the guest get the GPU?"
$G $LINUX /bin/sh -c 'lspci -nnk | grep -iA3 -E "VGA|Display|3D" | head -12; echo "--- dri:"; ls -l /dev/dri/'

# Steam is an X11 client on XWayland. It needs the per-session Xwayland auth file as well as
# DISPLAY, and it has to run inside the user's own systemd manager or it exits after bootstrap
# with no keyring.
say "starting Steam in the desktop session"
$G $LINUX /bin/sh -c 'XA=$(ls -1 /run/user/1000/.mutter-Xwaylandauth.* 2>/dev/null | head -1); sudo -u ubuntu env XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus systemd-run --user --scope --setenv=DISPLAY=:0 --setenv=XAUTHORITY="$XA" --setenv=WAYLAND_DISPLAY=wayland-0 --setenv=XDG_SESSION_TYPE=wayland /usr/games/steam -silent > /tmp/steam-gpu.log 2>&1 & echo requested'
for _ in $(seq 1 30); do
    out="$($G $LINUX /bin/sh -c 'pgrep -u 1000 -x steam >/dev/null && echo up || echo down' 2>/dev/null)"
    [ "$out" = up ] && break
    sleep 6
done
echo "steam: ${out:-unknown}"

# Started in its own call: a pgrep guard sharing a command line with the loop's own path
# matches the shell running it, and the loop then silently never starts.
say "starting the loop"
$G $LINUX /bin/sh -c 'env JEV_GAME_READY_TIMEOUT=180 JEV_EPISODE_TIMEOUT=900 setsid nohup /usr/local/sbin/jev-loop.sh > /home/ubuntu/jev/jevrun.log 2>&1 < /dev/null & echo started'

say "give it two minutes, then run 04-verify-gpu.sh"
