// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::seed_v2_support::{
    PrivateDirectory, derive_once_request, open_store, request_bytes, runtime_counts,
    start_live_server,
};
use crate::management::{
    FileSeedDerivationKeyAuthority, SeedDerivationKeyAuthority, SeedKeyError, SeedKeyHandle,
    SeededRunSubmissionResponseV2, WorkflowRunRequestV2, WorkflowStore,
};

const MODE: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_MODE";
const ROLE: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_ROLE";
const DATABASE: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_DATABASE";
const REQUEST: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_REQUEST";
const RESPONSE: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_RESPONSE";
const KEYRING: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_KEYRING";
const CATALOG: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_CATALOG";
const BARRIER: &str = "HARNESS_SEED_V2_RESERVATION_PROCESS_BARRIER";

const CHILD_FILTER: &str = "seed_v2_reservation_process_child_entry";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(target_os = "linux")]
#[test]
fn independent_process_reservation_arbitration_converges_across_key_rotation() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("workflow.sqlite");
    drop(open_store(&database));

    let key_one = directory.keyring(
        "current-key-one.conf",
        "key-1",
        &[("key-1", "11"), ("key-2", "22")],
    );
    let key_two = directory.keyring(
        "current-key-two.conf",
        "key-2",
        &[("key-1", "11"), ("key-2", "22")],
    );
    let request_path = directory.path().join("request.json");
    let response_one = directory.path().join("response-key-one.json");
    let response_two = directory.path().join("response-key-two.json");
    let barrier = directory.path().join("selected-current-keys");
    fs::create_dir(&barrier).expect("create interprocess barrier directory");
    set_private_directory(&barrier);

    let (request, _) = derive_once_request();
    fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize shared request"),
    )
    .expect("write shared request");

    let ready_one = barrier.join("key-1.ready");
    let ready_two = barrier.join("key-2.ready");
    let first_child = OwnedChild::spawn(
        "key-1",
        &database,
        &request_path,
        &response_one,
        &key_one,
        &barrier,
    )
    .expect("spawn first independent process");
    let second_child = OwnedChild::spawn(
        "key-2",
        &database,
        &request_path,
        &response_two,
        &key_two,
        &barrier,
    )
    .expect("spawn second independent process");

    let ready = [ready_one, ready_two];
    let barrier_deadline = Instant::now() + PROCESS_TIMEOUT;
    if let Err(error) = wait_for_files(&ready, barrier_deadline) {
        panic!("independent key contenders did not reach the reservation barrier: {error}");
    }
    assert_eq!(
        fs::read_to_string(&ready[0]).expect("read first selected key"),
        "key-1"
    );
    assert_eq!(
        fs::read_to_string(&ready[1]).expect("read second selected key"),
        "key-2"
    );

    let first_output = first_child
        .finish()
        .unwrap_or_else(|error| panic!("first reservation process failed: {error}"));
    let second_output = second_child
        .finish()
        .unwrap_or_else(|error| panic!("second reservation process failed: {error}"));
    eprintln!(
        "H103 independent reservation race: child pid={} exit={:?}; child pid={} exit={:?}",
        first_output.process_id,
        first_output.output.status.code(),
        second_output.process_id,
        second_output.output.status.code()
    );
    assert_child_success(&first_output, "first rotated-key reservation contender");
    assert_child_success(&second_output, "second rotated-key reservation contender");

    let first: ChildReport = read_report(&response_one);
    let second: ChildReport = read_report(&response_two);
    assert_eq!(first.current_key_version, "key-1");
    assert_eq!(second.current_key_version, "key-2");
    assert_eq!(first.current_key_reads, 1);
    assert_eq!(second.current_key_reads, 1);
    assert!(first.pinned_key_reads > 0);
    assert!(second.pinned_key_reads > 0);
    assert_eq!(first.runtime_counts, (0, 0, 0, 0));
    assert_eq!(second.runtime_counts, (0, 0, 0, 0));

    let winner_version = first
        .response
        .seed_binding
        .key_version
        .as_deref()
        .expect("winner key version");
    assert!(winner_version == "key-1" || winner_version == "key-2");
    assert!(!first.pinned_key_versions.is_empty());
    assert!(!second.pinned_key_versions.is_empty());
    assert!(
        first
            .pinned_key_versions
            .iter()
            .all(|version| version == winner_version)
    );
    assert!(
        second
            .pinned_key_versions
            .iter()
            .all(|version| version == winner_version)
    );
    assert_eq!(
        first.response.seed_binding, second.response.seed_binding,
        "both independent processes must return the committed winner's binding"
    );
    assert_eq!(
        first.response.run.workflow_run_id,
        second.response.run.workflow_run_id
    );
    assert_eq!(
        first.response.seed_binding.operation_id,
        second.response.seed_binding.operation_id
    );
    assert_eq!(
        first.response.seed_binding.configuration_digest,
        second.response.seed_binding.configuration_digest
    );
    assert_eq!(
        first.response.seed_binding.effective_seed,
        second.response.seed_binding.effective_seed
    );

    let reopened = open_store(&database);
    let binding = reopened
        .read_seed_binding(&first.response.run.workflow_run_id)
        .expect("read durable winner")
        .expect("one durable seed binding");
    assert_eq!(
        binding.record().operation_id,
        first.response.seed_binding.operation_id
    );
    assert_eq!(
        binding.record().effective_seed,
        first.response.seed_binding.effective_seed
    );
    assert_eq!(
        binding
            .record()
            .derivation
            .as_ref()
            .map(|derivation| derivation.key.version.as_str()),
        Some(winner_version)
    );
    for table in [
        "management_seed_operations",
        "management_seed_bindings",
        "management_submissions",
        "management_runs",
        "management_events",
    ] {
        assert_eq!(row_count(&reopened, table), 1, "one durable {table} row");
    }
    drop(reopened);
    directory.cleanup();
}

#[cfg(target_os = "linux")]
#[test]
fn seed_v2_reservation_process_child_entry() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    assert_eq!(mode, "race", "unknown reservation child mode");
    let role = required_env(ROLE);
    assert!(role == "key-1" || role == "key-2");
    let barrier = PathBuf::from(required_env(BARRIER));
    let request: WorkflowRunRequestV2 =
        serde_json::from_slice(&fs::read(required_env(REQUEST)).expect("read parent request"))
            .expect("decode parent request");
    let authority = Arc::new(BarrierKeyAuthority::open(
        Path::new(&required_env(KEYRING)),
        &barrier,
        &role,
    ));
    let store = open_store(Path::new(&required_env(DATABASE)));
    let server = start_live_server(
        Arc::clone(&store),
        Arc::clone(&authority),
        &required_env(CATALOG),
    );
    let response = crate::management::ManagementClient::new(
        server.server.address(),
        super::super::seed_v2_support::TOKEN,
    )
    .expect("child process management client")
    .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
    .expect("submit contender request");
    assert_eq!(response.status, 200, "served response: {:?}", response.body);
    let response: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&response.body).expect("decode child response");
    assert_eq!(
        runtime_counts(&server.runtime_counters),
        (0, 0, 0, 0),
        "seed reservation arbitration must not invoke runtime effects"
    );

    let report = ChildReport {
        response,
        current_key_version: authority.current_key_version(),
        current_key_reads: authority.current_reads.load(Ordering::SeqCst),
        pinned_key_reads: authority.pinned_reads.load(Ordering::SeqCst),
        pinned_key_versions: authority
            .pinned_key_versions
            .lock()
            .expect("pinned key versions lock")
            .clone(),
        runtime_counts: runtime_counts(&server.runtime_counters),
    };
    server
        .server
        .shutdown()
        .expect("stop child management server");
    drop(store);
    fs::write(
        required_env(RESPONSE),
        serde_json::to_vec(&report).expect("serialize child report"),
    )
    .expect("write child report");
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ChildReport {
    response: SeededRunSubmissionResponseV2,
    current_key_version: String,
    current_key_reads: usize,
    pinned_key_reads: usize,
    pinned_key_versions: Vec<String>,
    runtime_counts: (usize, usize, usize, usize),
}

struct BarrierKeyAuthority {
    inner: FileSeedDerivationKeyAuthority,
    barrier: PathBuf,
    role: String,
    current_key_version: Mutex<String>,
    current_reads: AtomicUsize,
    pinned_reads: AtomicUsize,
    pinned_key_versions: Mutex<Vec<String>>,
}

impl BarrierKeyAuthority {
    fn open(path: &Path, barrier: &Path, role: &str) -> Self {
        let inner = FileSeedDerivationKeyAuthority::open(path).expect("protected test keyring");
        Self {
            inner,
            barrier: barrier.to_owned(),
            role: role.to_owned(),
            current_key_version: Mutex::new(String::new()),
            current_reads: AtomicUsize::new(0),
            pinned_reads: AtomicUsize::new(0),
            pinned_key_versions: Mutex::new(Vec::new()),
        }
    }

    fn current_key_version(&self) -> String {
        self.current_key_version
            .lock()
            .expect("current key version lock")
            .clone()
    }

    fn wait_for_peer_selection(&self, version: &str) -> Result<(), SeedKeyError> {
        let own = self.barrier.join(format!("{}.ready", self.role));
        let peer_role = if self.role == "key-1" {
            "key-2"
        } else {
            "key-1"
        };
        let peer = self.barrier.join(format!("{peer_role}.ready"));
        let temporary = self.barrier.join(format!("{}.ready.tmp", self.role));
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary)
            .map_err(|_| SeedKeyError::Unavailable)?;
        marker
            .write_all(version.as_bytes())
            .map_err(|_| SeedKeyError::Unavailable)?;
        marker.sync_all().map_err(|_| SeedKeyError::Unavailable)?;
        drop(marker);
        fs::rename(self.barrier.join(format!("{}.ready.tmp", self.role)), own)
            .map_err(|_| SeedKeyError::Unavailable)?;

        let deadline = Instant::now() + PROCESS_TIMEOUT;
        while !peer.exists() {
            if Instant::now() >= deadline {
                return Err(SeedKeyError::Unavailable);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

impl SeedDerivationKeyAuthority for BarrierKeyAuthority {
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError> {
        self.current_reads.fetch_add(1, Ordering::SeqCst);
        let key = self.inner.current_key()?;
        let version = key.identity().version.clone();
        *self
            .current_key_version
            .lock()
            .map_err(|_| SeedKeyError::Unavailable)? = version.clone();
        self.wait_for_peer_selection(&version)?;
        Ok(key)
    }

    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<SeedKeyHandle>, SeedKeyError> {
        self.pinned_reads.fetch_add(1, Ordering::SeqCst);
        self.pinned_key_versions
            .lock()
            .map_err(|_| SeedKeyError::Unavailable)?
            .push(version.to_owned());
        self.inner.key_for(authority_id, version)
    }
}

struct OwnedChild {
    process_id: u32,
    child: Option<Child>,
}

struct ChildOutput {
    process_id: u32,
    output: Output,
}

impl OwnedChild {
    fn spawn(
        role: &str,
        database: &Path,
        request: &Path,
        response: &Path,
        keyring: &Path,
        barrier: &Path,
    ) -> std::io::Result<Self> {
        let child = Command::new(std::env::current_exe()?)
            .arg(CHILD_FILTER)
            .arg("--nocapture")
            .env(MODE, "race")
            .env(ROLE, role)
            .env(DATABASE, database)
            .env(REQUEST, request)
            .env(RESPONSE, response)
            .env(KEYRING, keyring)
            .env(CATALOG, "live.catalog.rotation-race")
            .env(BARRIER, barrier)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self {
            process_id: child.id(),
            child: Some(child),
        });
    }

    fn finish(mut self) -> Result<ChildOutput, String> {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            match self
                .child
                .as_mut()
                .expect("child remains owned until wait completes")
                .try_wait()
            {
                Ok(Some(_)) => {
                    let output = self
                        .child
                        .take()
                        .expect("exited child remains owned")
                        .wait_with_output()
                        .map_err(|error| {
                            format!("collect child {} output: {error}", self.process_id)
                        })?;
                    return Ok(ChildOutput {
                        process_id: self.process_id,
                        output,
                    });
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => return Err(self.terminate("child exceeded its 30s deadline")),
                Err(error) => {
                    return Err(self.terminate(&format!("cannot observe child: {error}")));
                }
            }
        }
    }
}

impl OwnedChild {
    fn terminate(&mut self, reason: &str) -> String {
        let Some(mut child) = self.child.take() else {
            return format!("child {} {reason}", self.process_id);
        };
        let _ = child.kill();
        match child.wait_with_output() {
            Ok(output) => format!(
                "child {} {reason}; status={:?}; stdout={}; stderr={}",
                self.process_id,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => format!("child {} {reason}; reap failed: {error}", self.process_id),
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for_files(paths: &[PathBuf], deadline: Instant) -> Result<(), String> {
    while !paths.iter().all(|path| path.is_file()) {
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for key selections {paths:?}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn assert_child_success(child: &ChildOutput, label: &str) {
    assert!(
        child.output.status.success(),
        "{label} child {} failed with {:?}; stdout={}; stderr={}",
        child.process_id,
        child.output.status.code(),
        String::from_utf8_lossy(&child.output.stdout),
        String::from_utf8_lossy(&child.output.stderr)
    );
    let stdout = String::from_utf8_lossy(&child.output.stdout);
    assert!(
        stdout.lines().any(|line| line == "running 1 test"),
        "{label} child {} did not report one selected test: {stdout}",
        child.process_id
    );
    let selected_test = stdout
        .lines()
        .filter(|line| line.contains("::seed_v2_reservation_process_child_entry ... ok"))
        .count();
    assert_eq!(
        selected_test, 1,
        "{label} child {} did not pass the reservation child entry exactly once: {stdout}",
        child.process_id
    );
    let summaries = stdout
        .lines()
        .filter(|line| line.starts_with("test result:"))
        .collect::<Vec<_>>();
    assert!(
        summaries.len() == 1
            && summaries[0].starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"),
        "{label} child {} did not report one passing, non-ignored test: {stdout}",
        child.process_id
    );
    eprintln!(
        "{label} child {} stdout={stdout}; stderr={}",
        child.process_id,
        String::from_utf8_lossy(&child.output.stderr)
    );
}

fn read_report(path: &Path) -> ChildReport {
    serde_json::from_slice(&fs::read(path).expect("read child process report"))
        .expect("decode child process report")
}

fn row_count(store: &crate::management::SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_seed_bindings"
            | "management_submissions"
            | "management_runs"
            | "management_events"
    ));
    store
        .connection
        .lock()
        .expect("SQLite connection lock")
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("read durable row count")
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing child process input {name}"))
}

#[cfg(target_os = "linux")]
fn set_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .expect("protect interprocess barrier directory");
}

#[cfg(not(target_os = "linux"))]
fn set_private_directory(_path: &Path) {}
