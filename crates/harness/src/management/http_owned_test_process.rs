// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

pub(super) const ENTRY: &str = "management::http::owned_process_tests::isolated_http_entry";
pub(super) const ROOT: &str = "STS2_MANAGEMENT_OWNED_TEST_ROOT";
pub(super) const ADDRESS: &str = "STS2_MANAGEMENT_OWNED_TEST_ADDRESS";
pub(super) const LEAF: &str = "STS2_MANAGEMENT_OWNED_TEST_LEAF";
pub(super) const TOKEN: &str = "owned-fixture-private-bearer";

pub(super) fn entry_command(root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("executable"));
    command
        .args(["--exact", ENTRY, "--nocapture", "--test-threads=1"])
        .env(ROOT, root)
        .env_remove(owned_backend::BACKEND_ROLE);
    command
}

pub(super) fn entry() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(root) = std::env::var(ROOT) else {
        return Ok(());
    };
    if std::env::var_os(LEAF).is_some() {
        loop {
            thread::park();
        }
    }
    let root = PathBuf::from(root);
    if owned_management_supervisor_requested() {
        service::record(&root, "supervisor", &std::process::id().to_string());
        if owned_supervisor::run().is_err() {
            std::process::exit(2);
        }
        return Ok(());
    }
    if owned_management_backend_requested() {
        let (service, auth) = service::compose(&root)?;
        return owned_backend::run(service, auth).map_err(Into::into);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let control_stop = Arc::clone(&stop);
    let control = thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        control_stop.store(true, Ordering::Release);
    });
    let limits = HttpLimits {
        dispatch_deadline: Duration::from_millis(
            std::env::var("STS2_MANAGEMENT_OWNED_TEST_BUDGET")?.parse()?,
        ),
        deadline: Duration::from_millis(500),
        max_connections: 4,
        ..HttpLimits::default()
    };
    let result = owned_frontend::serve_with_stop(std::env::var(ADDRESS)?.parse()?, limits, &stop);
    // Driver closes stdin on both normal shutdown and a frontend fault.
    if stop.load(Ordering::Acquire) {
        control.join().expect("control thread");
    }
    result.map_err(Into::into)
}

pub(super) struct Fixture {
    pub root: PathBuf,
    pub address: SocketAddr,
    child: Child,
    blocked_stderr: Option<UnixStream>,
}

impl Fixture {
    pub(super) fn start(block_stderr: bool) -> Self {
        Self::start_with_budget(block_stderr, 250)
    }

    pub(super) fn start_with_budget(block_stderr: bool, budget_millis: u64) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/owned-http-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).expect("test output directory");
        let listener = TcpListener::bind("127.0.0.1:0").expect("address reservation");
        let address = listener.local_addr().expect("address");
        let mut command = entry_command(&root);
        command
            .env(ADDRESS, address.to_string())
            .env(
                "STS2_MANAGEMENT_OWNED_TEST_BUDGET",
                budget_millis.to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        let blocked_stderr = if block_stderr {
            let (mut writer, reader) = UnixStream::pair().expect("diagnostic pipe");
            writer.set_nonblocking(true).expect("nonblocking fill");
            loop {
                match writer.write(&[b'x'; 8192]) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    result => assert!(result.expect("fill diagnostic pipe") > 0),
                }
            }
            writer.set_nonblocking(false).expect("blocking sink");
            let fd: OwnedFd = writer.into();
            command.stderr(Stdio::from(fd));
            Some(reader)
        } else {
            command.stderr(Stdio::from(
                fs::File::create(root.join("stderr")).expect("diagnostic log"),
            ));
            None
        };
        drop(listener);
        let child = command.spawn().expect("frontend process");
        let mut fixture = Self {
            root,
            address,
            child,
            blocked_stderr,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect_timeout(&address, Duration::from_millis(20)).is_ok() {
                break;
            }
            assert!(
                fixture.child.try_wait().expect("frontend status").is_none(),
                "frontend exited during startup: {}",
                fixture.logs()
            );
            assert!(
                Instant::now() < deadline,
                "frontend startup deadline: {}",
                fixture.logs()
            );
            thread::sleep(Duration::from_millis(5));
        }
        fixture
    }

    pub(super) fn request(&self, method: &str, path: &str, body: Option<&[u8]>) -> ClientResponse {
        let client = ManagementClient::new(self.address, TOKEN).expect("client");
        client
            .request_json(method, path, body)
            .expect("bounded HTTP response")
    }

    pub(super) fn logs(&self) -> String {
        fs::read_to_string(self.root.join("stderr")).unwrap_or_default()
    }

    pub(super) fn diagnostic_loss(&self) -> u64 {
        let mut stream = TcpStream::connect(self.address).expect("HTTP peer");
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .expect("read bound");
        stream
            .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("request");
        let mut response = String::new();
        stream
            .take(65_536)
            .read_to_string(&mut response)
            .expect("bounded response including loss header");
        assert!(response.starts_with("HTTP/1.1 200"));
        response
            .lines()
            .find_map(|line| line.strip_prefix("X-Ascension-Diagnostic-Loss: "))
            .expect("explicit diagnostic loss header")
            .trim()
            .parse()
            .expect("loss count")
    }

    pub(super) fn process(&self, record: &str, index: usize) -> OwnedFd {
        let records = service::recorded(&self.root, record);
        let pid = records
            .get(index)
            .expect("recorded process")
            .parse::<i32>()
            .expect("PID");
        pidfd_open(
            Pid::from_raw(pid).expect("nonzero PID"),
            PidfdFlags::empty(),
        )
        .expect("retain exact process identity")
    }

    pub(super) fn wait_records(&mut self, name: &str, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while service::recorded(&self.root, name).len() < count {
            assert!(
                self.child.try_wait().expect("frontend status").is_none(),
                "frontend exited: {}",
                self.logs()
            );
            assert!(Instant::now() < deadline, "missing test witness {name}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(mut input) = self.child.stdin.take() {
            let _ = input.write_all(b"stop\n");
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = self.child.try_wait().expect("frontend status") {
                assert!(status.success(), "frontend failed: {}", self.logs());
                return;
            }
            assert!(
                Instant::now() < deadline,
                "frontend failed to reap its helper"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut input) = self.child.stdin.take() {
            let _ = input.write_all(b"stop\n");
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            // Only the directly owned frontend is killed. Closing its inherited IPC makes the
            // dedicated helper drain the backend; the test host never performs wait(All).
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.blocked_stderr = None;
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn assert_exited(fd: &OwnedFd) {
    let mut descriptors = [PollFd::new(fd, PollFlags::IN)];
    poll(
        &mut descriptors,
        Some(&rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    )
    .expect("pidfd poll");
    assert!(
        descriptors[0].revents().contains(PollFlags::IN),
        "original process must have exited"
    );
}

pub(super) fn await_exited(fd: &OwnedFd) {
    let mut descriptors = [PollFd::new(fd, PollFlags::IN)];
    poll(
        &mut descriptors,
        Some(&rustix::event::Timespec {
            tv_sec: 3,
            tv_nsec: 250_000_000,
        }),
    )
    .expect("bounded pidfd wait");
    assert!(
        descriptors[0].revents().contains(PollFlags::IN),
        "diagnostic budget plus actual reap must finish within the declared bound"
    );
}
