// SPDX-License-Identifier: MIT

use super::fixture;
use super::{EffectCompletion, EffectHandle, EffectPort, LifecycleError, SendPermit};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BARRIER_LIMIT: Duration = Duration::from_secs(60);

pub(super) fn expected_completion() -> EffectCompletion {
    fixture::Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("fixture response poll")
    .expect("fixture completion")
}

pub(super) fn assert_one_effect_attempt(root: &Path) {
    let attempts = fs::read(root.join("effect-attempts.log")).expect("effect counter");
    assert_eq!(attempts, b"effect-start\n");
}

pub(super) fn assert_sent_journal_unchanged(root: &Path) {
    let expected =
        fs::read_to_string(root.join("sent-journal.sha256")).expect("sent journal receipt");
    let current = crate::sha256_hex(
        fs::read(root.join("owner").join("journal.enc")).expect("journal still sent"),
    );
    assert_eq!(current, expected);
}

pub(super) fn assert_response_delivered(root: &Path) {
    let expected = expected_completion();
    let receipt = format!(
        "{} {}\n",
        expected.response.len(),
        crate::sha256_hex(&expected.response)
    );
    assert_eq!(
        fs::read(root.join("response-delivered.txt")).expect("delivered response receipt"),
        receipt.as_bytes()
    );
}

pub(super) fn assert_response_not_delivered(root: &Path) {
    assert!(
        !root.join("response-delivered.txt").exists(),
        "response receipt exists before effect handle delivery"
    );
}

pub(super) struct PersistentProcessEffect {
    root: PathBuf,
    pre_response_marker: Option<PathBuf>,
}

impl PersistentProcessEffect {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            pre_response_marker: None,
        }
    }

    pub(super) fn stop_before_handle_at(mut self, marker: PathBuf) -> Self {
        self.pre_response_marker = Some(marker);
        self
    }
}

pub(super) struct PersistentProcessHandle {
    inner: fixture::Handle,
    receipt: PathBuf,
}

impl EffectPort for PersistentProcessEffect {
    type Handle = PersistentProcessHandle;

    fn try_start(&mut self, _: SendPermit, _: &[u8]) -> Result<Self::Handle, LifecycleError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("effect-attempts.log"))
            .map_err(|_| LifecycleError::Io)?;
        file.write_all(b"effect-start\n")
            .and_then(|()| file.sync_all())
            .map_err(|_| LifecycleError::Io)?;
        if let Some(marker) = self.pre_response_marker.take() {
            mark_possible_write_and_wait(&self.root, &marker);
        }
        Ok(PersistentProcessHandle {
            inner: fixture::Handle {
                ready: true,
                units: Some(3),
            },
            receipt: self.root.join("response-delivered.txt"),
        })
    }
}

impl EffectHandle for PersistentProcessHandle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError> {
        let completion = self.inner.poll()?;
        if let Some(completion) = &completion {
            let receipt = format!(
                "{} {}\n",
                completion.response.len(),
                crate::sha256_hex(&completion.response)
            );
            fixture::write_process_file(&self.receipt, receipt.as_bytes());
        }
        Ok(completion)
    }
}

fn mark_possible_write_and_wait(root: &Path, marker: &Path) -> ! {
    let sent_journal = fs::read(root.join("owner").join("journal.enc"))
        .expect("sent journal before effect handle");
    fixture::write_process_file(
        &root.join("sent-journal.sha256"),
        crate::sha256_hex(sent_journal).as_bytes(),
    );
    fixture::write_process_file(marker, b"effect attempt synced before handle delivery\n");
    let deadline = Instant::now() + BARRIER_LIMIT;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("parent did not kill child at the pre-response barrier")
}
