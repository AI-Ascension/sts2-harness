// SPDX-License-Identifier: MIT

    #[cfg(test)]
    mod tests {
        use super::*;
        use rustix::pipe::{PipeFlags, pipe_with};
        use serde_json::json;
        use std::fs::File;
        use std::io::Write;
        use std::thread;
        use std::time::Duration;

        fn valid_payload() -> Vec<u8> {
            serde_json::to_vec(&json!({
                "version": 1,
                "launch_nonce": "12345678-1234-4234-8234-123456789abc",
                "watchdog_boot_id": "22345678-1234-4234-8234-123456789abc",
                "component_id": "harness",
                "expected_peer": {
                    "platform": "linux",
                    "pid": 1,
                    "creation_token": "1",
                    "executable": "/bin/true",
                    "executable_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "uid": 0,
                    "gid": 0
                }
            }))
            .unwrap_or_default()
        }

        #[test]
        fn bootstrap_decoder_rejects_unknown_and_duplicate_fields() {
            assert!(parse_bootstrap_payload(&valid_payload()).is_ok());
            let unknown = br#"{"version":1,"launch_nonce":"12345678-1234-4234-8234-123456789abc","watchdog_boot_id":"22345678-1234-4234-8234-123456789abc","component_id":"harness","expected_peer":{},"extra":1}"#;
            assert!(parse_bootstrap_payload(unknown).is_err());
            let duplicate = br#"{"version":1,"version":1,"launch_nonce":"12345678-1234-4234-8234-123456789abc","watchdog_boot_id":"22345678-1234-4234-8234-123456789abc","component_id":"harness","expected_peer":{}}"#;
            assert!(parse_bootstrap_payload(duplicate).is_err());
        }

        #[test]
        fn constant_time_comparison_requires_exact_bytes() {
            assert!(constant_time_equal(b"worker", b"worker"));
            assert!(!constant_time_equal(b"worker", b"worker2"));
            assert!(!constant_time_equal(b"worker", b"workEr"));
        }

        #[test]
        fn bootstrap_reader_rejects_delayed_trailing_bytes()
        -> Result<(), Box<dyn std::error::Error>> {
            let (reader_fd, writer_fd) = pipe_with(PipeFlags::NONBLOCK)?;
            let mut reader = File::from(reader_fd);
            let mut writer = File::from(writer_fd);
            let payload = valid_payload();
            let mut frame = BOOTSTRAP_MAGIC.to_vec();
            frame.extend_from_slice(&u32::try_from(payload.len())?.to_be_bytes());
            frame.extend_from_slice(&payload);
            writer.write_all(&frame)?;
            let delayed = thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                writer.write_all(b"delayed").is_ok()
            });
            let result = read_bootstrap_from(&mut reader, Duration::from_millis(250));
            assert!(matches!(
                result,
                Err(message) if message == "worker bootstrap has trailing bytes"
            ));
            assert!(delayed.join().is_ok_and(|written| written));
            Ok(())
        }

        #[test]
        fn authentication_slots_apply_a_hard_bound() {
            let mut slots = AuthSlotBudget::default();
            for _ in 0..MAX_AUTH_SLOTS {
                assert!(slots.try_acquire());
            }
            assert!(!slots.try_acquire());
            slots.release();
            assert!(slots.try_acquire());
            slots.release();
        }

        #[test]
        fn runtime_image_snapshot_survives_path_and_in_place_replacement()
        -> Result<(), Box<dyn std::error::Error>> {
            let directory = std::env::temp_dir().join(format!(
                "sts2-worker-runtime-image-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&directory)?;
            let path = directory.join("runtime");
            std::fs::copy("/bin/true", &path)?;
            let digest = {
                let mut source = File::open(&path)?;
                hash_file(&mut source)?
            };
            let approved = verify_executable(&path, Some(&digest))?;
            std::fs::write(&path, std::fs::read("/bin/false")?)?;
            std::fs::rename(&path, directory.join("runtime.approved"))?;
            std::fs::copy("/bin/false", &path)?;
            let status = std::process::Command::new(approved.command_path()).status()?;
            assert!(status.success(), "snapshot followed the replaced pathname");
            std::fs::remove_dir_all(directory)?;
            Ok(())
        }
    }
