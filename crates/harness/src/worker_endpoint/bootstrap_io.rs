// SPDX-License-Identifier: MIT

    fn read_bootstrap() -> Result<Bootstrap, String> {
        let descriptor = open(
            "/proc/self/fd/0",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| String::from("worker bootstrap stdin is unavailable"))?;
        let mut reader: File = descriptor.into();
        read_bootstrap_from(&mut reader, BOOTSTRAP_TIMEOUT)
    }

    fn read_bootstrap_from(reader: &mut File, timeout: Duration) -> Result<Bootstrap, String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| String::from("worker bootstrap deadline overflow"))?;
        let mut prefix = [0_u8; BOOTSTRAP_PREFIX_BYTES];
        read_exact_deadline(reader, &mut prefix, deadline, "worker bootstrap")?;
        if &prefix[..BOOTSTRAP_MAGIC.len()] != BOOTSTRAP_MAGIC {
            return Err(String::from("worker bootstrap magic is invalid"));
        }
        let payload_len =
            u32::from_be_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]) as usize;
        if payload_len == 0 || payload_len > BOOTSTRAP_MAX_PAYLOAD {
            return Err(String::from("worker bootstrap payload length is invalid"));
        }
        let mut payload = vec![0_u8; payload_len];
        read_exact_deadline(reader, &mut payload, deadline, "worker bootstrap")?;
        read_until_eof_deadline(reader, deadline)?;
        parse_bootstrap_payload(&payload)
    }

    fn read_until_eof_deadline(reader: &mut File, deadline: Instant) -> Result<(), String> {
        let mut trailing = [0_u8; 256];
        loop {
            if Instant::now() >= deadline {
                return Err(String::from("worker bootstrap EOF timed out"));
            }
            match reader.read(&mut trailing) {
                Ok(0) => return Ok(()),
                Ok(_) => return Err(String::from("worker bootstrap has trailing bytes")),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(POLL_INTERVAL);
                }
                Err(_) => return Err(String::from("worker bootstrap could not be completed")),
            }
        }
    }

    fn parse_bootstrap_payload(payload: &[u8]) -> Result<Bootstrap, String> {
        let value = crate::worker_handoff::json::decode(payload)
            .map_err(|_| String::from("worker bootstrap JSON is invalid"))?;
        let object = value
            .as_object()
            .ok_or_else(|| String::from("worker bootstrap schema is invalid"))?;
        exact_fields(
            object,
            &[
                "version",
                "launch_nonce",
                "watchdog_boot_id",
                "component_id",
                "expected_peer",
            ],
        )?;
        if object.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(String::from("worker bootstrap version is unsupported"));
        }
        let launch_nonce = uuid4_string(object, "launch_nonce")?;
        let watchdog_boot_id = uuid4_string(object, "watchdog_boot_id")?;
        let component_id = object
            .get("component_id")
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("worker bootstrap component is invalid"))?
            .to_owned();
        validate_component(&component_id)?;
        let peer_object = object
            .get("expected_peer")
            .and_then(Value::as_object)
            .ok_or_else(|| String::from("worker bootstrap peer is invalid"))?;
        exact_fields(
            peer_object,
            &[
                "platform",
                "pid",
                "creation_token",
                "executable",
                "executable_sha256",
                "uid",
                "gid",
            ],
        )?;
        if peer_object.get("platform").and_then(Value::as_str) != Some("linux") {
            return Err(String::from("worker bootstrap peer platform is invalid"));
        }
        let pid = bounded_u32(peer_object, "pid", true)?;
        let uid = bounded_u32(peer_object, "uid", false)?;
        let gid = bounded_u32(peer_object, "gid", false)?;
        let creation_token = peer_string(peer_object, "creation_token")?.to_owned();
        validate_creation_token(&creation_token)?;
        let executable_text = peer_string(peer_object, "executable")?;
        validate_absolute_path(executable_text, "worker executable")?;
        let executable_sha256 = peer_string(peer_object, "executable_sha256")?.to_owned();
        validate_digest(&executable_sha256)
            .map_err(|_| String::from("worker executable digest is invalid"))?;
        Ok(Bootstrap {
            launch_nonce,
            watchdog_boot_id,
            component_id,
            peer: LinuxPeer {
                pid,
                creation_token,
                executable: PathBuf::from(executable_text),
                executable_sha256,
                uid,
                gid,
            },
        })
    }
