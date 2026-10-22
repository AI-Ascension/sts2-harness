// SPDX-License-Identifier: MIT

//! The dedicated child supervisor never constructs user ports or touches their journals.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use super::owned_backend::BACKEND_ROLE;
use super::owned_deadline::Deadline;
use super::owned_process_scope::ChildScope;
use super::owned_wire::{Frames, FromBackend, ToBackend, WIRE_VERSION};
use super::*;

#[path = "http_owned_supervisor_state.rs"]
mod state;
use state::Supervisor;

pub(super) const REAP_BUDGET: Duration = Duration::from_secs(2);
const START_BUDGET: Duration = Duration::from_secs(30);
const DIAGNOSTIC_BUDGET: Duration = Duration::from_secs(1);
const MAX_DIAGNOSTICS: usize = 64;

pub(super) struct Generation {
    number: u64,
    child: Child,
    frames: Frames,
    startup: Deadline,
    ready: bool,
    retiring: bool,
}

impl Generation {
    fn spawn(number: u64, capacity: usize) -> Result<Self, HttpError> {
        let (socket, inherited) = UnixStream::pair().map_err(io_http_error)?;
        let descriptor: OwnedFd = inherited.into();
        let mut command = same_binary("backend")?;
        command.stdin(Stdio::from(descriptor));
        let mut frames = Frames::new(socket).map_err(|_| fault("owned_backend_channel"))?;
        frames
            .queue(&ToBackend::Hello {
                version: WIRE_VERSION,
                generation: number,
                capacity,
            })
            .map_err(|_| fault("owned_backend_handshake"))?;
        let startup = Deadline::after(START_BUDGET).ok_or_else(|| fault("owned_backend_clock"))?;
        // No fallible operation follows spawn until the child is stored in the owned generation.
        let child = command.spawn().map_err(|_| fault("owned_backend_spawn"))?;
        Ok(Self {
            number,
            child,
            frames,
            startup,
            ready: false,
            retiring: false,
        })
    }
}

pub(super) fn same_binary(role: &'static str) -> Result<Command, HttpError> {
    let executable = std::env::current_exe().map_err(io_http_error)?;
    let mut command = Command::new(executable);
    command
        .args(std::env::args_os().skip(1))
        .env(BACKEND_ROLE, role)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    Ok(command)
}

/// Called only by the same binary's hidden supervisor role, before any user composition.
/// The embedding host never enables subreaper and never performs an all-child wait.
pub(super) fn run() -> Result<(), HttpError> {
    if std::env::var(BACKEND_ROLE).as_deref() != Ok("supervisor") {
        return Err(fault("owned_supervisor_role"));
    }
    super::owned_child_wait::require_waitable_children()?;
    // This dedicated, re-executed helper never runs embedding-host callbacks. Its panic
    // reporting cannot touch stderr: a blocked default hook would precede catch_unwind and
    // prevent descendant draining. The catch reports a fixed typed failure after actual reap;
    // neither a private panic payload nor an inherited blocking sink is invoked here.
    std::panic::set_hook(Box::new(|_| {}));
    let socket = inherited_socket()?;
    let mut parent = Frames::new(socket).map_err(|_| fault("owned_supervisor_channel"))?;
    let (generation, capacity) = hello(&mut parent)?;
    let scope = ChildScope::enable().map_err(|_| fault("owned_supervisor_scope"))?;
    let mut supervisor = Supervisor::new(scope, parent, generation, capacity);
    // No user callback runs in this process. Still retain child ownership through an internal
    // panic: the error path performs the same actual reap before this helper is allowed to exit.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| supervisor.serve()));
    supervisor.drain_before_exit();
    let result = result.unwrap_or_else(|_| Err(fault("owned_supervisor_panicked")));
    if let Err(error) = &result {
        supervisor.report_failure(&error.code);
    }
    result
}

pub(super) fn inherited_socket() -> Result<UnixStream, HttpError> {
    let socket = UnixStream::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| fault("owned_supervisor_channel"))?,
    );
    if !socket.local_addr().map_err(io_http_error)?.is_unnamed()
        || !socket.peer_addr().map_err(io_http_error)?.is_unnamed()
    {
        return Err(fault("owned_supervisor_channel"));
    }
    Ok(socket)
}

fn hello(frames: &mut Frames) -> Result<(u64, usize), HttpError> {
    let deadline = Deadline::after(START_BUDGET).ok_or_else(|| fault("owned_supervisor_clock"))?;
    loop {
        match frames
            .receive()
            .map_err(|_| fault("owned_supervisor_handshake"))?
        {
            Some(ToBackend::Hello {
                version: WIRE_VERSION,
                generation,
                capacity,
            }) if generation != 0 && (1..=MAX_CONNECTIONS).contains(&capacity) => {
                return Ok((generation, capacity));
            }
            Some(_) => return Err(fault("owned_supervisor_handshake")),
            None if deadline.expired() => return Err(fault("owned_supervisor_handshake")),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

fn fault(code: &'static str) -> HttpError {
    HttpError::from_management(ManagementError::unavailable(
        code,
        "owned management supervision is unavailable",
    ))
}
