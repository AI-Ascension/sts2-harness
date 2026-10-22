// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::management::http::owned_deadline::Deadline;
use crate::management::http::owned_wire::{Frames, FromBackend, ToBackend, WIRE_VERSION};
use std::fs;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Stdio};

struct Helper {
    child: Child,
    channel: Option<Frames>,
    root: PathBuf,
    blocked_sink: Option<UnixStream>,
    backend_identity: Option<OwnedFd>,
}

impl Helper {
    fn start(block_stderr: bool) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/owned-http-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).expect("test directory");
        let (socket, inherited) = UnixStream::pair().expect("private channel");
        let descriptor: OwnedFd = inherited.into();
        let mut command = entry_command(&root);
        command
            .env(owned_backend::BACKEND_ROLE, "supervisor")
            .stdin(Stdio::from(descriptor))
            .stdout(Stdio::null());
        let blocked_sink = if block_stderr {
            let (mut output, input) = UnixStream::pair().expect("stderr pipe");
            output.set_nonblocking(true).expect("nonblocking fill");
            loop {
                match output.write(&[b'x'; 8192]) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    result => {
                        assert!(result.expect("fill stderr") > 0);
                    }
                }
            }
            output.set_nonblocking(false).expect("blocked stderr");
            let descriptor: OwnedFd = output.into();
            command.stderr(Stdio::from(descriptor));
            Some(input)
        } else {
            command.stderr(Stdio::from(
                fs::File::create(root.join("stderr")).expect("stderr"),
            ));
            None
        };
        let frames = Frames::new(socket).expect("frames");
        let child = command.spawn().expect("supervisor");
        let mut helper = Self {
            child,
            channel: Some(frames),
            root,
            blocked_sink,
            backend_identity: None,
        };
        helper.send(&ToBackend::Hello {
            version: WIRE_VERSION,
            generation: 1,
            capacity: 4,
        });
        assert!(matches!(
            helper.receive(),
            FromBackend::Ready {
                version: WIRE_VERSION,
                generation: 1
            }
        ));
        helper.backend_identity = Some(helper.backend());
        helper
    }

    fn send(&mut self, frame: &ToBackend) {
        let channel = self.channel.as_mut().expect("open channel");
        channel.queue(frame).expect("queue frame");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !channel.output_empty() {
            channel.flush().expect("flush frame");
            assert!(Instant::now() < deadline, "private writer deadline");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn receive(&mut self) -> FromBackend {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(message) = self
                .channel
                .as_mut()
                .expect("open channel")
                .receive()
                .expect("private reply")
            {
                return message;
            }
            assert!(Instant::now() < deadline, "private reply deadline");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn backend(&self) -> OwnedFd {
        use rustix::process::{Pid, PidfdFlags, pidfd_open};
        let pid: i32 = service::recorded(&self.root, "backend")[0]
            .parse()
            .expect("backend PID");
        pidfd_open(Pid::from_raw(pid).expect("PID"), PidfdFlags::empty()).expect("backend pidfd")
    }

    fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("helper status") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "helper did not finish owned cleanup"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.channel = None;
        self.blocked_sink = None;
        // Closing the private parent channel normally drives the real drain even after an
        // assertion fails. The bounded fallback below is limited to these childless probes.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            // These protocol probes send only health/invalid frames; their service cannot
            // spawn descendants. Retain a pidfd before probing, so assertion cleanup can stop
            // that exact backend independently of the supervisor implementation under test.
            if let Some(backend) = &self.backend_identity {
                let _ = rustix::process::pidfd_send_signal(backend, rustix::process::Signal::KILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn health(generation: u64, sequence: u64) -> ToBackend {
    ToBackend::Dispatch {
        generation,
        sequence,
        deadline_nanos: Deadline::after(Duration::from_secs(2))
            .expect("deadline")
            .wire(),
        request: HttpRequest {
            method: "GET".to_owned(),
            path: "/v1/health".to_owned(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
        .into(),
    }
}

#[test]
fn stale_generation_and_replayed_sequence_never_start_a_second_handler() {
    for replay_sequence in [false, true] {
        let mut helper = Helper::start(false);
        let backend = helper.backend();
        if replay_sequence {
            helper.send(&health(1, 1));
            assert!(matches!(
                helper.receive(),
                FromBackend::Response {
                    sequence: 1,
                    status: 200,
                    ..
                }
            ));
            helper.send(&health(1, 1));
        } else {
            helper.send(&health(99, 1));
        }
        assert!(
            matches!(helper.receive(), FromBackend::SupervisorFailed { generation: 1, code } if code == "owned_parent_identity")
        );
        assert!(!helper.wait().success());
        assert_exited(&backend);
    }
}

#[test]
fn a_helper_panic_with_blocked_stderr_still_reaps_before_reporting_typed_failure() {
    let mut helper = Helper::start(true);
    let backend = helper.backend();
    let started = Instant::now();
    helper.send(&ToBackend::PanicForTest { generation: 1 });
    assert!(
        matches!(helper.receive(), FromBackend::SupervisorFailed { generation: 1, code } if code == "owned_supervisor_panicked")
    );
    assert!(!helper.wait().success());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_exited(&backend);
}
