# Giving the Jev Linux guest a slice of the Arc B60

Staged 2026-09-19. **Nothing here has been run.** The Linux domain, the game and the loop are
untouched; every step below is written down and ready, and step 1 is the first one that stops
anything.

## Why

The guest renders on the CPU today. Its only graphics device is virtio-gpu, which gives OpenGL
through virgl but no Vulkan, and the game is Godot 4 Forward+, which is Vulkan-only. Vulkan over
virtio-gpu needs venus, and the host's virglrenderer is 1.0.0 without it. So Mesa falls back to
lavapipe and the game burns 362% of one core of four, sustained, on a turn-based card game.

The card is already doing SR-IOV. It just was never asked for a second slice:

    sriov_totalvfs = 7
    sriov_numvfs   = 1
    0000:07:00.1 -> xe-vfio-pci -> <hostdev> on sts.home.complete.tech-windows

The guest is already equipped to use one: the `xe` module is present and Mesa 25.2.8 ships
`intel_icd.json`, so a VF gives it native Intel Vulkan instead of software.

## Order

| Step | Script | Stops anything? |
| --- | --- | --- |
| 0 | `00-preflight.sh` | No - read-only, run it first and again before step 1 |
| 1 | `01-create-vf.sh` | Yes - the **Windows** domain, for about a minute |
| 2 | `02-attach-vf.sh` | No - `--config` only, the running Linux domain is untouched |
| 3 | `03-restart-lane.sh` | Yes - **this is the one that stops the game** |
| 4 | `04-verify-gpu.sh` | No |
| - | `99-rollback.sh` | Yes - both domains, puts the card back to one VF |

Run them as a user that can write `/sys/bus/pci/...`, so under `sudo`, from this directory.
Steps 1 and 99 check that themselves and refuse before stopping anything, because failing the
sysfs write after a domain is already down is the worst outcome available here.

Both scripts also record which domains were running when they started and put back only those.
Neither will quietly start a domain that was deliberately left off.

## What each step is working around

**Step 1.** The kernel will not change `sriov_numvfs` while a VF is assigned, and will not go from
1 to 2 without passing through 0. VF0 belongs to the running Windows domain, so that domain has to
be down for the duration. The script shuts it down gracefully, never forcibly, and aborts before
touching the card if it will not stop. It also aborts if VF0 does not come back at `0000:07:00.1`,
because the Windows domain's `<hostdev>` names that address.

**Step 2.** `virsh attach-device --config` edits the persistent definition only. This is why it is
safe to run while the lane is playing - but do not run it before step 1, or the next start of the
domain fails on a host device that does not exist.

**Step 3.** Nothing about the lane comes back on its own after a cold start, and two things bite:

- Steam is an X11 client on XWayland. It needs `XAUTHORITY` pointing at the per-session
  `/run/user/1000/.mutter-Xwaylandauth.*` file, not just `DISPLAY`, or it bootstraps and then exits
  with `Unable to open X11 display`. It also has to run inside the user's own systemd manager
  (`systemd-run --user --scope`) or it exits after bootstrap with no keyring.
- The loop is started in its own call. A `pgrep -f "[j]ev-loop.sh"` guard sharing a command line
  with the unbracketed `/usr/local/sbin/jev-loop.sh` start command matches the shell running it, so
  the loop silently never starts.

**Step 4.** Success is the game log saying an Intel device instead of
`Using Device #0: Unknown - llvmpipe`, and the CPU sample dropping well below 362%.

## The one real risk

Godot picks its own device and the reviewed launcher fixes argv, so `--gpu-index` is not available.
With both the Arc VF and lavapipe present it should choose the Arc - Godot scores a real GPU above
a CPU one - but if it picks wrong, the `Using Device #0:` line in step 4 says so immediately, and
`99-rollback.sh` puts everything back.

## Persistence

`sriov-vfs.service` pins the VF count across reboots. Nothing on this host sets it today - there is
no systemd unit and no udev rule - so the current Windows passthrough is already one reboot away
from losing its VF. Install it after step 1 succeeds:

    sudo cp sriov-vfs.service /etc/systemd/system/
    sudo systemctl daemon-reload
    sudo systemctl enable sriov-vfs.service     # do not start it; step 1 already did the work

## Backups

`backup/` holds the inactive XML of both domains as they were before any of this.

## What has been tested

`tests/run-tests.sh` runs the two steps that stop something against a mock host - a fake sysfs
tree and a fake `virsh` whose domain states are files - so it is safe to run anywhere, as an
unprivileged user, with no libvirt and no GPU. Nine cases pass:

- the happy path reaches two VFs and puts Windows back
- a Windows domain that was already off stays off
- a host already at two VFs is a no-op that never stops anything
- an unwritable `sriov_numvfs` refuses **before** stopping Windows
- a Windows domain that will not stop aborts without touching the card
- a missing VF1 after provisioning fails but still restarts Windows
- rollback returns to one VF and restarts both domains
- rollback leaves an already-off domain off
- a domain that will not stop aborts the rollback, and both domains are still put back

Every path that can fail after a domain has been stopped restarts it. That was the defect the
first draft shipped with, and the fourth test above is there to keep it fixed.

Against the live host, with throwaway probe domains that were defined and then removed:

- libvirt accepts `hostdev-vf1.xml` and auto-assigns the guest PCI address, so the file
  deliberately carries no `<address>` of its own
- `virsh detach-device --config` with that same file matches and removes the device, which is
  what `99-rollback.sh` relies on: one hostdev before, zero after

The scripts take `JEV_VIRSH`, `JEV_GPU_DEVROOT`, `JEV_GUEST_EXEC`, `JEV_LINUX_DOMAIN` and
`JEV_WINDOWS_DOMAIN` from the environment, defaulting to this host. That is what lets the tests
point them at a mock rather than carrying a second copy of the logic.

Static analysis is clean: `shellcheck -S warning` reports nothing across all six steps and the
test suite. Fixing what it did report removed a real hazard in `00-preflight.sh`, which used
`test && ok || bad` for each check - a form where the failure branch also runs if the success
branch returns non-zero, which is the one thing a preflight must never do quietly.
