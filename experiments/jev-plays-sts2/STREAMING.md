# Linux streaming and benchmark sessions

`jev-loop.sh benchmark` (also the no-argument default) retains bounded episodes, native process
teardown, and a fresh profile for each benchmark episode. `jev-loop.sh stream` launches the native
game once and omits the shell's harness timeout. Both modes share a supervisor lock. Streaming
refuses to kill or adopt an already-running game.

Streaming requires the separately installed `launch-linux-native-rest-stream.py` beside the
reviewed native launcher in the operator bundle. `streaming-launcher.patch` records its narrow
change against original SHA-256
`5d1cfc718b6d17c1678af7b523f3eea81f7a25d1427a9da5704e87c621e46520`:
only explicit `--persistent-session` disables duration-based game termination. Default duration
bounds, executable identity checks, profile preparation, stop-file handling, process-exit handling,
and owned-process cleanup are retained. Preserve the original launcher for benchmarking. Do not
apply the patch to another launcher revision without reviewing its differences.

The first streaming start still uses the authorized isolated fresh profile. Subsequent operator
resumes retain that same live game process and profile; they do not reopen or restore a saved run.
Closing the game or placing `stop` in its first run directory ends the native session. Closing OBS
is independent of this lifetime. No OBS settings are changed.

The supervisor records the first streaming run directory in `stream-session.path` under the Jev
installation. On harness completion or failure it leaves the game visible and pauses, retaining
the exact outcome and existing durable evidence. It makes no automatic provider retry. After
resolving the stop, the operator may place `resume` in that first directory to start a new harness
segment against the same live game. Pending/unknown effects must be reconciled first; resuming is
not proof that a failed action did not happen.

**Automatic win/loss-to-new-game progression is unavailable in the installed host catalog.**
The existing host reports terminal observations with no legal return-to-menu action. This mode
does not invent such an action, inject game UI input, reset saves, or repeatedly allocate terminal
episodes. The operator must return to the main menu before requesting the next run. Adding a
supported terminal transition remains separate game-mod/harness work.

The desktop session must enable `jev-fullscreen@complete.tech` and have
`org.gnome.shell disable-user-extensions` set to `false`. Its existing compositor helper keeps the
game fullscreen and restores focus; it does not require restarting GNOME or the VM. Verify the
actual compositor state after launch rather than inferring it from settings files.

Validation: `node --test experiments/jev-plays-sts2/session-modes.test.mjs` runs command-double
checks for benchmark teardown, streaming success/failure/timeout retention, and explicit resume
without native process/profile replacement. Those checks launch no game or provider. The native
launcher variant additionally passed the existing fourteen dry-run fixture checks and five
parser/deadline checks on the authorized Linux guest. Live evidence must be reported separately.
