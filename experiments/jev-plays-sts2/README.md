# Jev plays Slay the Spire 2

The operator scripts that run a TypeSafe System One model (`jev-latest`) against a live game, one
episode at a time, restarting the whole session after each one.

They live here because they were lost once. They existed only in a scratch directory and on the two
guests, an editing mistake truncated all three copies at the same moment, and there was nothing to
restore from. Anything that is the only copy of itself is one mistake from gone.

## What is here

| file | lane |
| --- | --- |
| `jev-loop.sh` | Linux guest. Starts the game through the reviewed launcher, which loads the mod. |
| `jev-loop.ps1` | Windows guest. Starts the game directly, because there is no launcher for it. |
| `systemone_transport.ps1` | Windows transport the bridge spawns for one provider exchange. |

Two more files belong with these and are **not** here: `systemone_transport.py`, the Linux transport,
and `jev-context.py`, which reads what either transport recorded. Both are Python, which `LANG001`
prohibits across this organization's repositories, so they remain operator-local -- which is exactly
the condition that lost the Windows loop. Porting them to a permitted language would close that.

The two loops are kept in step: the same credentials per session, the same gateway and harness
identity, the same bounds. Where they differ it is because the platforms differ, and the comment in
the script says why.

## What they expect

Both read `key.txt` beside the script for the provider credential. It is never an argument, never
logged, and must not be placed in any directory that is served over HTTP during deployment.

Each episode generates its own runtime and gateway credentials from the operating system CSPRNG.
Nothing is read from a saved setting and no token has to be known in advance.

## Reading a run

Every episode writes its own directory under `runs/`, holding the game log, the gateway and harness
output, the outcome, and `jev-context.jsonl` -- every exchange with the provider, including the
state, the instructions, every option with its description, and the answer.

```text
python3 jev-context.py <log>            every exchange, one line each
python3 jev-context.py <log> 2          exchange 2 in full
python3 jev-context.py <log> 2 --state  just the state
python3 jev-context.py <log> --options  per option: times shown, times top, average probability
```

## Settings that were arrived at by measurement, not taste

- **Confidence gate 20.** At 35 the model starts a run and picks a map node but refuses every combat
  answer: its calibrated confidence in combat sits near 0.25, so it abstains and re-observes without
  acting.
- **`STS2_CAMPAIGN_EPISODE`, not `STS2_COMBAT_DEMO`.** The combat demo only acts once the host is
  already in combat and never leaves a menu, so against a freshly launched game it polls an
  unchanging main menu and the model is asked nothing.
- **Barrier 40 x 3s.** The default 8 x 1s expires while the game is still loading and fails the
  episode before the first decision.
- **The gateway is given the harness's own instance identity and a token scope.** Without the scope
  allocation is answered with HTTP 401; with a different instance, HTTP 409.

## Known host-side blockers

- The offered set at a reward carries no information, and a skipped reward is offered again:
  AI-Ascension/sts2-game-mod#171.
- No action continues a saved run, so every episode starts over:
  AI-Ascension/sts2-game-mod#172.
- On Windows the mod refuses to initialise unless its user directory holds a fresh profile baseline:
  AI-Ascension/sts2-game-mod#173.

## The isolated user directory the Windows lane declares

The mod also refuses to initialise unless the directory the *game* resolved equals the directory the
*launcher* published in `STS2_LIVE_USER_DIR`, and the game resolves that directory from
`config/custom_user_dir_name` in `override.cfg` under the launch-scoped `APPDATA` root -- not from
any launcher argument. The Linux lane satisfies this through the reviewed launcher, which sets the
variable from `--user-dir-mapping`; the Windows lane starts the host executable directly, so the
variable is its own responsibility.

It was never set, so every episode that reached the mod failed with `live demo requires its isolated
user directory` while the game was resolving exactly the directory the lane had isolated -- the
message names neither directory, which is why the episode looked like a directory problem. The lane
now reads the directory back out of the `override.cfg` it just wrote, seeds that value, and declares
that same value, so the two cannot drift apart; a resolver failure is recorded as an episode failure
rather than silently becoming a different directory. The test
`crates/harness/tests/jev_loop_windows_user_dir_declaration.rs` fails if the declaration is removed
or stops being the resolved value.

## The live runtime the Windows lane opts into

The mod's production live runtime is opt-in. `STS2_LIVE_COMBAT=1` is the switch that binds the
gameplay host at all; with it, `STS2_LIVE_CAMPAIGN=1` selects the isolated campaign save backend and
`STS2_LIVE_CAMPAIGN_MODE=standard` selects the standard seeded run path. The reviewed launcher sets
all three in the child environment, and the Linux lane inherits them that way. The Windows lane
launches the host executable directly, so the environment is its own responsibility, and it declared
only the runtime session variables.

The failure that produced was not a refusal. The mod loaded, printed
`authenticated runtime HTTP listener started on 127.0.0.1:15626`, and served every read as
`{"state":"recovery","code":"host_not_configured"}` -- the code its host seam reports when no host
was ever configured. The harness read that as an unknown state, re-observed once, and ended the
episode as `episode requires recovery before policy can continue`, four seconds after the game
reached its main menu, with two checkpoints and no operation. Nothing in that message says
"environment", which is why the listener evidence looked like progress.

The lane now declares all three before starting the game. They are constants rather than per-episode
values, so they are not in the episode's credential cleanup list. The test
`crates/harness/tests/jev_loop_windows_live_campaign_env.rs` fails if any of them is removed,
renamed, given another value, or declared after the game has already been started.

## Fullscreen on the Linux guest

The reviewed launcher starts the game with a fixed `--windowed --resolution 958x699`, refuses to run
with an ambient `DISPLAY`, and refuses to pass one to the game, so the game is a Wayland client that
can never be fullscreened from outside by the usual X tools. Three things were tried and do not
work, recorded here so they are not tried again:

- **A `fullscreen` flag in the profile baseline.** The game rewrites `live-campaign/settings.save`
  from the mapped settings file before it applies display settings, so the copied value is
  discarded. It also costs a re-record of the launcher's 96-file / 1768384-byte baseline, which the
  launcher pins as a literal.
- **A `fullscreen` flag in the settings template.** The mapped file is the mod settings file; the
  game strips everything but `mod_settings` when it writes it back, and the command line wins over
  the file for both size and mode - the settings ask for 1280x720 and the window comes up 958x699.
- **Injecting the compositor's fullscreen keybinding.** `ydotoold` is not in Ubuntu's `ydotool`
  0.1.8, and the transient uinput device the client creates on its own is never added to the seat,
  so Mutter never sees the key. `org.gnome.Shell.Eval` is closed on GNOME 46.

What does work is `gnome-fullscreen-extension/`, a GNOME Shell extension that fullscreens and
focuses the game window from inside the compositor. Install it as the desktop user:

    D=~/.local/share/gnome-shell/extensions/jev-fullscreen@complete.tech
    mkdir -p "$D" && cp gnome-fullscreen-extension/* "$D"/
    gnome-extensions enable jev-fullscreen@complete.tech

On Wayland the shell only picks up a newly installed extension at session start, so restart the
session once (`systemctl restart gdm`; autologin brings it back). It has to focus the window as well
as fullscreen it: Mutter only hides the top bar and the dock for the *focused* fullscreen window,
and nothing in this lane ever clicks the game.

Restarting the session gives the desktop a new login session with a new leader PID, which used to
fail every following episode with "live loginctl properties differ from the authenticated session
record". The loop now re-records `session-env.json` from the live session before each launch.

Steam has to come up inside the desktop session or it exits during bootstrap without a keyring -
`sudo -u ubuntu ... setsid /usr/games/steam` is not enough. Start it in the user's own manager:

    sudo -u ubuntu env XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        systemd-run --user --scope --setenv=DISPLAY=:0 --setenv=WAYLAND_DISPLAY=wayland-0 \
        /usr/games/steam -silent

### Keeping it fullscreen

Making the window fullscreen once is not enough. The launcher passes `--windowed`, and the game
applies its own saved display settings *after* the window is mapped - the log line is
`[Display] Attempting WINDOWED mode` - which takes it straight back out of fullscreen. The symptom
is a window that looks nearly right and still has the Ubuntu top bar over it: the compositor
reported `fs=false` on a `1024x805+0+0` window against a 1024x768 screen. The extension therefore
watches `notify::fullscreen` on the game window and re-asserts, bounded to 30 attempts so a build
that genuinely refuses cannot spin. It re-asserts focus on `notify::focus-window` for the same
reason: Mutter only hides the top bar and the dock for the *focused* fullscreen window.

Two things make this hard to debug, so the extension logs the window stack itself:

- `org.gnome.Shell.Eval` and `org.gnome.Shell.Introspect.GetWindows` are both refused on GNOME 46,
  so there is no external way to ask what is on top.
- The shell only imports an extension's code at session start. `gnome-extensions disable` then
  `enable` re-runs `enable()` on the module already in memory, it does **not** pick up an edited
  file - a changed extension needs `systemctl restart gdm`. That does make the toggle a cheap way
  to dump the current window stack, which is what `_dump('enable')` is for:

      journalctl -b -o cat | grep -F 'jev-fullscreen enable:'
      jev-fullscreen enable: 1 windows [Slay the Spire 2|...|fs=true|focus=true|1024x768+0+0]

Steam also needs `XAUTHORITY`, not just `DISPLAY`: it is an X11 client on XWayland, and without the
auth file it logs `Authorization required, but no authorization protocol specified` and
`Unable to open X11 display, exiting` after bootstrapping. The file is per session, so resolve it:

    XA=$(ls -1 /run/user/1000/.mutter-Xwaylandauth.* | head -1)
    sudo -u ubuntu env XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        systemd-run --user --scope --setenv=DISPLAY=:0 --setenv=XAUTHORITY="$XA" \
        --setenv=WAYLAND_DISPLAY=wayland-0 /usr/games/steam -silent

Finally, do not guard the loop start with `pgrep -f "[j]ev-loop.sh"` in the same command line that
starts `/usr/local/sbin/jev-loop.sh`. The bracket keeps the *pattern* from matching itself, but the
unbracketed path in the start command is on that same command line, so the guard matches the shell
running it and the loop never starts. Check in one call and start in another.

## Rendering on the CPU

The guest renders in software. Its only graphics device is virtio-gpu, which gives OpenGL through
virgl but no Vulkan, and the game is Godot 4 Forward+, which is Vulkan-only - Vulkan over
virtio-gpu needs venus, and the host's virglrenderer is 1.0.0 without it. So Mesa falls back to
lavapipe and the game sustains 362% of one core of four for a turn-based card game:

    Vulkan 1.4.318 - Forward+ - Using Device #0: Unknown - llvmpipe (LLVM 20.1.2, 256 bits)

The frame rate is not capped either. `fps_limit` is 60 in the settings file, but the game takes its
effective settings from the mapped file and scavenges it as a version-0 save, so the saved values
are replaced by defaults - the same reason it ignores `fullscreen`. Vsync cannot pick up the slack
because Mutter 46 does not implement the Wayland `fifo-v1` protocol, which the game reports as
`FIFO protocol not found! Frame pacing will be degraded`.

The host has an Intel Arc Pro B60 already doing SR-IOV, with one of seven virtual functions given
to the Windows domain and six free. `gpu-passthrough/` holds a prepared, unexecuted plan to give
one to this guest, with a read-only preflight, a rollback, and the reasoning for each step.
