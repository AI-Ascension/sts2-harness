// SPDX-License-Identifier: MIT

//! The parent half of the #673 announce-after-bind protocol.
//!
//! `free_address()` used to pick a loopback port by binding `:0`, reading the assigned port
//! back and **dropping the listener**, and the stub was then told to bind that port itself.
//! Between the drop and the stub's own `bind` the port is unowned, so a parallel test in the
//! same binary can be handed it and be serving it. The stub then dies at its `bind` with
//! `EADDRINUSE` while `ready()` — which only requires a TCP connect to succeed — has already
//! connected to the *foreign* listener and reported the gateway ready. That is the `held == 0`
//! shape from #629, and it is the reason the capture assertions could be fooled by a squatted
//! address rather than only by a real truncation.
//!
//! Holding the port in the parent does **not** fix this, and that is worth recording because it
//! is the obvious thing to try. The stub has to `bind` the address itself, and a second
//! concurrent `LISTEN` on one `addr:port` is refused by the kernel no matter what the options
//! are: with no options, with `SO_REUSEADDR`, with `SO_REUSEPORT`, and with both, the second bind
//! fails `EADDRINUSE` (errno 98). `SO_REUSEADDR` only relaxes the `TIME_WAIT` rule for a port
//! whose previous listener is *already* closed; it never permits two simultaneous listeners.
//! A parent-held listener therefore does not reserve the port for the child, it blocks the child
//! from taking it.
//!
//! So the port is not chosen in the parent at all. The stub binds `127.0.0.1:0`, which the
//! kernel allocates atomically, and **announces** the address it actually got. The port is owned
//! continuously from allocation to service and there is no window to lose, with no reservation
//! protocol and no lock to inherit across the process boundary.
//!
//! The announcement path is derived from the stub's own location rather than passed as another
//! environment variable, because the shell wrapper already knows it as `$0`. That keeps the
//! spawn path in `runtime_v4_executable_composition_process.rs` — which is already at
//! `rust_test_preferred` — completely untouched, and it means every stub gets the protocol
//! without each one having to opt in.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// The suffix the stub appends to its own path when announcing its address.
///
/// Kept as one constant so the shell side (`"$0.announced"`, written by the stub wrapper) and
/// this parent side cannot drift into disagreeing about where the file is.
const SUFFIX: &str = "announced";

/// How long a stub gets to bind `:0` and announce before the scenario gives up.
///
/// Generous rather than tight on purpose: the wait covers a `python3` start plus a bind, and a
/// test host that is merely loaded should not be reported as a defect in the protocol. Expiry is
/// still a hard failure that names the path, so a stub that never announces cannot hang the
/// binary or be mistaken for a slow-but-fine one.
pub(crate) const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the stub running at `stub` announces the address it bound.
pub(crate) fn announcement_path(stub: &Path) -> PathBuf {
    let mut name = stub.as_os_str().to_os_string();
    name.push(".");
    name.push(SUFFIX);
    PathBuf::from(name)
}

/// The address a stub announced, waiting for it to appear and to be written in full.
///
/// The stub writes the file *after* `listen`, so once this returns the port is already owned by
/// the child and serving — which is the property the old shape could not offer at any point.
/// Polls rather than reading once because the announcement is written by a separate process a
/// moment after spawn, and re-parses rather than trusting the first byte because the parent can
/// observe the file mid-write; a partially written address simply does not parse and is retried.
pub(crate) fn announced_address(
    stub: &Path,
    timeout: Duration,
) -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let path = announcement_path(stub);
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    loop {
        match fs::read_to_string(&path) {
            Ok(text) => {
                last = text.trim().to_owned();
                if let Ok(address) = last.parse::<SocketAddr>() {
                    return Ok(address);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Box::new(error)),
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the gateway never announced a bound address at {} within {timeout:?}; it must \
                 bind :0 and announce, so that no port is ever unowned (sts2-harness#673). Last \
                 read: {last:?}",
                path.display()
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}
