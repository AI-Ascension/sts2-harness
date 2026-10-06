// SPDX-License-Identifier: MIT

use super::fixture::write_process_file as write_once;
use super::fixture::{self, Fixture};
use super::{
    PersistentProcessEffect as PersistentEffect,
    assert_one_process_effect_attempt as assert_one_effect_attempt,
    assert_process_response_was_delivered as assert_response_was_delivered,
    assert_process_sent_journal_unchanged as assert_sent_journal_unchanged,
    expected_process_completion as expected_completion,
};
use crate::exo_lifecycle::{LifecyclePhase, StartOutcome};
use crate::provider_session::owner_journal::inject_terminal_process_barrier;
use crate::{ExecutionStore, ProviderReservationState};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CUT_ENV: &str = "STS2_H142_PROCESS_CRASH_CUT";
const ROOT_ENV: &str = "STS2_H142_PROCESS_CRASH_ROOT";
const CHILD_TEST: &str =
    "exo_lifecycle::tests::process_recovery_tests::child_process_stops_at_requested_crash_cut";
const CHILD_WAIT: Duration = Duration::from_secs(20);
const OUTPUT_DIAGNOSTIC_LIMIT: usize = 4096;

#[derive(Clone, Copy)]
enum CrashCut {
    BeforeResultCommit,
    BeforeTerminalJournalCommit,
}

impl CrashCut {
    fn env_value(self) -> &'static str {
        match self {
            Self::BeforeResultCommit => "before-result-commit",
            Self::BeforeTerminalJournalCommit => "before-terminal-journal-commit",
        }
    }

    fn marker_text(self) -> &'static [u8] {
        match self {
            Self::BeforeResultCommit => b"result transaction updates reached before commit\n",
            Self::BeforeTerminalJournalCommit => b"terminal journal commit reached\n",
        }
    }

    fn marker_path(self, root: &Path) -> PathBuf {
        root.join(format!("{}.arrived", self.env_value()))
    }
}

fn open_existing_fixture(root: PathBuf) -> Fixture {
    let manifest: super::InvocationManifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).expect("fixture manifest"))
            .expect("decode fixture manifest");
    let input = fs::read(root.join("input.bin")).expect("fixture input");
    assert_eq!(manifest.input_length, input.len());
    assert_eq!(manifest.input_digest, crate::sha256_hex(&input));
    let initial_broker = fixture::broker();
    let policy = initial_broker.policy().clone();
    let capabilities = initial_broker.capabilities().clone();
    let config = super::JournalConfig {
        directory: root.join("owner"),
        legacy_path: None,
        store_id: "journal-fixture".into(),
        scope: manifest.scope.clone(),
        owner_binding_digest: crate::sha256_hex("authenticated-owner-fixture"),
    };
    let store = ExecutionStore::open(crate::ExecutionStoreConfig::new(
        root.join("execution.sqlite3"),
    ))
    .expect("existing store");
    let fingerprint =
        crate::ExecutionFingerprint::new("seed", "build", "state", "config", "provider")
            .expect("fingerprint");
    Fixture {
        root,
        config,
        manifest,
        input,
        broker: None,
        policy,
        capabilities,
        authority: std::sync::Arc::new(fixture::Authority::default()),
        store,
        fingerprint,
        cleanup_on_drop: false,
    }
}

#[test]
fn process_death_before_result_commit_rolls_back_and_stays_held() {
    run_process_cut(CrashCut::BeforeResultCommit);
}

#[test]
fn process_death_after_result_commit_recovers_without_another_effect() {
    run_process_cut(CrashCut::BeforeTerminalJournalCommit);
}

#[test]
fn child_process_stops_at_requested_crash_cut() {
    let Some(cut_name) = std::env::var_os(CUT_ENV) else {
        return;
    };
    let cut = match cut_name.to_str() {
        Some("before-result-commit") => CrashCut::BeforeResultCommit,
        Some("before-terminal-journal-commit") => CrashCut::BeforeTerminalJournalCommit,
        _ => panic!("unknown process crash cut"),
    };
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).expect("private fixture root"));
    let mut fixture = Fixture::new_at(root.clone());
    write_once(
        &root.join("manifest.json"),
        &serde_json::to_vec(&fixture.manifest).expect("manifest bytes"),
    );
    write_once(&root.join("input.bin"), &fixture.input);
    let mut owner = fixture.owner();
    let mut effect = PersistentEffect::new(&root);
    let outcome = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("one synthetic effect start");
    let StartOutcome::Started(mut handle) = outcome else {
        panic!("fresh child must own the started handle");
    };
    let sent_journal =
        fs::read(fixture.config.directory.join("journal.enc")).expect("sent journal bytes");
    write_once(
        &root.join("sent-journal.sha256"),
        crate::sha256_hex(sent_journal).as_bytes(),
    );

    match cut {
        CrashCut::BeforeResultCommit => {
            ExecutionStore::inject_result_commit_process_barrier(cut.marker_path(&root))
        }
        CrashCut::BeforeTerminalJournalCommit => {
            inject_terminal_process_barrier(cut.marker_path(&root));
        }
    }
    let unexpected = owner.poll(&mut handle, &mut fixture.store);
    panic!("parent did not kill child at the armed cut: {unexpected:?}");
}

fn run_process_cut(cut: CrashCut) {
    let mut private_root = PrivateRoot::new();
    let mut child = OwnedChild::spawn(&private_root.path, &private_root.state, cut);
    child.wait_for_marker(cut.marker_path(&private_root.state), cut.marker_text());
    assert_sent_journal_unchanged(&private_root.state);
    assert_one_effect_attempt(&private_root.state);
    assert_response_was_delivered(&private_root.state);
    let status = child.kill_and_reap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(9),
        "the exact child was not SIGKILLed"
    );
    child.assert_captured_output();

    let mut fixture = open_existing_fixture(private_root.state.clone());
    let mut owner = fixture.reopen().expect("new owner after child death");
    match cut {
        CrashCut::BeforeResultCommit => assert_precommit_recovery(&mut fixture, &mut owner),
        CrashCut::BeforeTerminalJournalCommit => {
            assert_postcommit_recovery(&mut fixture, &mut owner)
        }
    }
    assert_one_effect_attempt(&private_root.state);
    drop(owner);
    drop(fixture);
    private_root.cleanup();
}

fn assert_precommit_recovery(
    fixture: &mut Fixture,
    owner: &mut crate::exo_lifecycle::LifecycleOwner,
) {
    let decision = fixture
        .store
        .decision(&fixture.manifest.execution_id)
        .expect("decision row survives rollback");
    assert!(!decision.completed);
    assert!(!decision.unknown);
    assert!(decision.result_payload.is_none());
    let reservation = fixture
        .store
        .provider_reservation(&fixture.manifest.reservation_id)
        .expect("reservation row survives rollback");
    assert_eq!(reservation.state, ProviderReservationState::Reserved);
    assert_eq!(owner.entries().len(), 1);
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Unknown);
    assert!(owner.entries()[0].possible_write);
    assert!(
        owner
            .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
            .is_err()
    );
    let mut no_second_effect = PersistentEffect::new(&fixture.root);
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut no_second_effect,
            )
            .is_err()
    );
}

fn assert_postcommit_recovery(
    fixture: &mut Fixture,
    owner: &mut crate::exo_lifecycle::LifecycleOwner,
) {
    let expected = expected_completion();
    let expected_digest = crate::sha256_hex(&expected.response);
    let stored = fixture
        .store
        .decision(&fixture.manifest.execution_id)
        .expect("durable decision");
    assert!(stored.completed);
    assert!(!stored.unknown);
    assert_eq!(
        stored.result_payload.as_deref(),
        Some(expected.response.as_slice())
    );
    assert_eq!(
        stored.reference.result_digest.as_deref(),
        Some(expected_digest.as_str())
    );

    let expected_decision =
        super::super::validation::response(&fixture.manifest, &fixture.input, &expected.response)
            .expect("expected exact decision");
    let recovered = owner
        .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
        .expect("recover exact result in new owner");
    assert_eq!(recovered, expected_decision);
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Completed);
    assert_eq!(
        owner.entries()[0].result_digest.as_deref(),
        Some(expected_digest.as_str())
    );
    let repeated = owner
        .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
        .expect("idempotent exact result recovery");
    assert_eq!(repeated, expected_decision);
    let mut no_second_effect = PersistentEffect::new(&fixture.root);
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut no_second_effect,
            )
            .is_err()
    );
}

struct OwnedChild {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
    reaped: bool,
}

impl OwnedChild {
    fn spawn(output_root: &Path, state_root: &Path, cut: CrashCut) -> Self {
        let stdout = output_root.join("child.stdout");
        let stderr = output_root.join("child.stderr");
        let stdout_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stdout)
            .expect("exclusive child stdout");
        let stderr_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stderr)
            .expect("exclusive child stderr");
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env_remove(CUT_ENV)
            .env_remove(ROOT_ENV)
            .env(CUT_ENV, cut.env_value())
            .env(ROOT_ENV, state_root)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .expect("spawn exact child test");
        Self {
            child,
            stdout,
            stderr,
            reaped: false,
        }
    }

    fn wait_for_marker(&mut self, marker: PathBuf, expected: &[u8]) {
        let deadline = Instant::now() + CHILD_WAIT;
        while Instant::now() < deadline {
            if marker.is_file() {
                assert_eq!(fs::read(&marker).expect("barrier marker"), expected);
                return;
            }
            if let Some(status) = self.child.try_wait().expect("child status") {
                self.reaped = true;
                panic!(
                    "child exited before barrier: {status}; fixture {}\n{}",
                    marker.parent().expect("private root").display(),
                    child_diagnostics(self.stdout.parent().expect("child output root"))
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = self.kill_and_reap();
        panic!(
            "child missed barrier deadline and was reaped ({status}); fixture {}\n{}",
            marker.parent().expect("private root").display(),
            child_diagnostics(self.stdout.parent().expect("child output root"))
        );
    }

    fn kill_and_reap(&mut self) -> ExitStatus {
        let _ = self.child.kill();
        let status = self.child.wait().expect("reap exact child process");
        self.reaped = true;
        status
    }

    fn assert_captured_output(&self) {
        for path in [&self.stdout, &self.stderr] {
            let size = fs::metadata(path).expect("captured child stream").len();
            assert!(size <= 64 * 1024, "child output exceeded its bound");
        }
    }
}

fn bounded_stream(path: &Path) -> String {
    let size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    let mut bytes = Vec::new();
    let _ = fs::File::open(path).and_then(|file| {
        file.take((OUTPUT_DIAGNOSTIC_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
    });
    let shown = bytes.len().min(OUTPUT_DIAGNOSTIC_LIMIT);
    format!("{size} bytes: {}", String::from_utf8_lossy(&bytes[..shown]))
}

fn child_diagnostics(root: &Path) -> String {
    format!(
        "child.stdout [{}]; child.stderr [{}]",
        bounded_stream(&root.join("child.stdout")),
        bounded_stream(&root.join("child.stderr"))
    )
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
    }
}

struct PrivateRoot {
    path: PathBuf,
    state: PathBuf,
    cleaned: bool,
}

impl PrivateRoot {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "sts2-h142-process-{}-{nanos}-{nonce}",
            std::process::id()
        ));
        assert!(!path.exists(), "fixture root must be absent");
        let mut builder = fs::DirBuilder::new();
        builder
            .mode(0o700)
            .create(&path)
            .expect("private output root");
        Self {
            state: path.join("state"),
            path,
            cleaned: false,
        }
    }

    fn cleanup(&mut self) {
        fs::remove_dir_all(&self.path).expect("remove private process fixture");
        self.cleaned = true;
    }
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        if !self.cleaned {
            eprintln!(
                "preserved synthetic process fixture {}\n{}",
                self.state.display(),
                child_diagnostics(&self.path)
            );
        }
    }
}
