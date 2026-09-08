// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn verifier_entry_precedes_configuration_and_rejects_extra_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    for extra in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command.env_clear().arg("--worker-peer-verifier-v1");
        if extra {
            command.arg("unexpected");
        }
        // /dev/null is deliberately not the private sequenced-packet control socket.
        let mut child = OwnedChild(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.0.try_wait()? {
                assert_eq!(status.code(), Some(2));
                break;
            }
            assert!(Instant::now() < deadline, "verifier entry failed to exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut stderr = String::new();
        use std::io::Read;
        child
            .0
            .stderr
            .take()
            .ok_or("stderr missing")?
            .take(1024)
            .read_to_string(&mut stderr)?;
        assert_eq!(
            stderr,
            if extra {
                "worker peer verifier rejects extra arguments\n"
            } else {
                "worker peer verifier failed\n"
            }
        );
    }
    Ok(())
}
