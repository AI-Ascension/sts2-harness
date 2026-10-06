// SPDX-License-Identifier: MIT

use super::fixture::Fixture;
use super::fixture::write_process_file as write_once;
use super::process_effect_fixture::{
    PersistentProcessEffect as PersistentEffect, assert_one_effect_attempt,
    assert_response_delivered, assert_response_not_delivered, assert_sent_journal_unchanged,
};
use super::process_recovery_support::{
    assert_postcommit_recovery, assert_precommit_recovery, open_existing_fixture,
};
use crate::ExecutionStore;
use crate::exo_lifecycle::StartOutcome;
use crate::provider_session::owner_journal::inject_terminal_process_barrier;
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
    AfterPossibleWriteBeforeHandle,
}

impl CrashCut {
    fn env_value(self) -> &'static str {
        match self {
            Self::BeforeResultCommit => "before-result-commit",
            Self::BeforeTerminalJournalCommit => "before-terminal-journal-commit",
            Self::AfterPossibleWriteBeforeHandle => "after-possible-write-before-handle",
        }
    }

    fn marker_text(self) -> &'static [u8] {
        match self {
            Self::BeforeResultCommit => b"result transaction updates reached before commit\n",
            Self::BeforeTerminalJournalCommit => b"terminal journal commit reached\n",
            Self::AfterPossibleWriteBeforeHandle => {
                b"effect attempt synced before handle delivery\n"
            }
        }
    }

    fn marker_path(self, root: &Path) -> PathBuf {
        root.join(format!("{}.arrived", self.env_value()))
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
fn process_death_after_possible_write_before_handle_stays_held() {
    run_process_cut(CrashCut::AfterPossibleWriteBeforeHandle);
}

#[test]
fn child_process_stops_at_requested_crash_cut() {
    let Some(cut_name) = std::env::var_os(CUT_ENV) else {
        return;
    };
    let cut = match cut_name.to_str() {
        Some("before-result-commit") => CrashCut::BeforeResultCommit,
        Some("before-terminal-journal-commit") => CrashCut::BeforeTerminalJournalCommit,
        Some("after-possible-write-before-handle") => CrashCut::AfterPossibleWriteBeforeHandle,
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
    let effect = PersistentEffect::new(&root);
    let mut effect = if matches!(cut, CrashCut::AfterPossibleWriteBeforeHandle) {
        effect.stop_before_handle_at(cut.marker_path(&root))
    } else {
        effect
    };
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
    if !matches!(cut, CrashCut::AfterPossibleWriteBeforeHandle) {
        let sent_journal =
            fs::read(fixture.config.directory.join("journal.enc")).expect("sent journal bytes");
        write_once(
            &root.join("sent-journal.sha256"),
            crate::sha256_hex(sent_journal).as_bytes(),
        );
    }

    match cut {
        CrashCut::BeforeResultCommit => {
            ExecutionStore::inject_result_commit_process_barrier(cut.marker_path(&root))
        }
        CrashCut::BeforeTerminalJournalCommit => {
            inject_terminal_process_barrier(cut.marker_path(&root));
        }
        CrashCut::AfterPossibleWriteBeforeHandle => {
            panic!("pre-response effect barrier returned without parent kill")
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
    if matches!(cut, CrashCut::AfterPossibleWriteBeforeHandle) {
        assert_response_not_delivered(&private_root.state);
    } else {
        assert_response_delivered(&private_root.state);
    }
    let status = child.kill_and_reap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(9),
        "the exact child was not SIGKILLed"
    );
    child.assert_captured_output();
    if matches!(cut, CrashCut::AfterPossibleWriteBeforeHandle) {
        assert_response_not_delivered(&private_root.state);
    }

    let mut fixture = open_existing_fixture(private_root.state.clone());
    let mut owner = fixture.reopen().expect("new owner after child death");
    match cut {
        CrashCut::BeforeResultCommit | CrashCut::AfterPossibleWriteBeforeHandle => {
            assert_precommit_recovery(&mut fixture, &mut owner)
        }
        CrashCut::BeforeTerminalJournalCommit => {
            assert_postcommit_recovery(&mut fixture, &mut owner)
        }
    }
    assert_one_effect_attempt(&private_root.state);
    drop(owner);
    drop(fixture);
    private_root.cleanup();
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
