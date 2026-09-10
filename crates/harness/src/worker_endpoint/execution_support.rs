// SPDX-License-Identifier: MIT

    fn fresh_worker_boot_id(watchdog_boot_id: &str) -> String {
        loop {
            let value = uuid::Uuid::new_v4().to_string();
            if value != watchdog_boot_id {
                return value;
            }
        }
    }

    fn required_env(name: &str) -> Result<String, String> {
        optional_env(name)?.ok_or_else(|| format!("{name} is required"))
    }

    fn optional_env(name: &str) -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => Ok(Some(value)),
            Ok(_) => Err(format!("{name} must not be empty")),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not UTF-8")),
        }
    }

    fn alias_env(primary: &str, alias: &str) -> Result<Option<String>, String> {
        let primary_value = optional_env(primary)?;
        let alias_value = optional_env(alias)?;
        if primary_value.is_some() && alias_value.is_some() && primary_value != alias_value {
            return Err(format!("{primary} and {alias} disagree"));
        }
        Ok(primary_value.or(alias_value))
    }

    fn required_path(name: &str) -> Result<PathBuf, String> {
        let value = required_env(name)?;
        Ok(PathBuf::from(value))
    }

    fn optional_path(name: &str) -> Result<Option<PathBuf>, String> {
        optional_env(name).map(|value| value.map(PathBuf::from))
    }

    fn current_runtime_binary() -> Result<PathBuf, String> {
        let path = std::env::current_exe()
            .map_err(|_| String::from("worker runtime executable is unavailable"))?;
        // The Linux watchdog helper executes approved images from a sealed
        // memfd.  In that case `/proc/self/exe` is the only stable executable
        // reference; the visible `/memfd:... (deleted)` symlink is not a
        // pathname that can be reopened.  The proc self reference remains
        // bound to this exact image for the lifetime of the endpoint.
        if path.to_string_lossy().starts_with("/memfd:")
            || path.to_string_lossy().ends_with(" (deleted)")
        {
            Ok(PathBuf::from("/proc/self/exe"))
        } else {
            Ok(path)
        }
    }

    #[derive(Clone)]
    struct ApprovedExecutable {
        /// The endpoint executes this sealed snapshot through its descriptor,
        /// never by reopening the configured pathname after validation.
        image: std::sync::Arc<File>,
    }

    impl ApprovedExecutable {
        fn command_path(&self) -> PathBuf {
            PathBuf::from(format!("/proc/self/fd/{}", self.image.as_raw_fd()))
        }
    }

    fn verify_executable(
        path: &Path,
        expected: Option<&str>,
    ) -> Result<ApprovedExecutable, String> {
        let mut source = if path == Path::new("/proc/self/exe") {
            File::open(path)
                .map_err(|_| String::from("worker runtime executable is unavailable"))?
        } else {
            validate_reference(path, "worker runtime executable")?;
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!("worker runtime executable is unavailable: metadata {error}")
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(String::from("worker runtime executable is not immutable"));
            }
            let canonical = fs::canonicalize(path).map_err(|error| {
                format!("worker runtime executable is unavailable: canonicalize {error}")
            })?;
            if canonical != path {
                return Err(String::from("worker runtime executable is not canonical"));
            }
            open(
                path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|error| {
                format!("worker runtime executable is unavailable: open {error}")
            })?
        };
        let metadata = fstat(&source)
            .map_err(|_| String::from("worker runtime executable is unavailable"))?;
        if (metadata.st_mode & 0o170_000) != 0o100_000
            || metadata.st_mode & 0o111 == 0
            || (path != Path::new("/proc/self/exe") && metadata.st_mode & 0o022 != 0)
        {
            return Err(String::from("worker runtime executable is not immutable"));
        }
        let snapshot_fd = create_executable_snapshot()?;
        let mut snapshot: File = snapshot_fd.into();
        let digest = hash_and_copy(&mut source, &mut snapshot)?;
        if let Some(expected) = expected {
            validate_digest(expected)
                .map_err(|_| String::from("worker runtime digest is invalid"))?;
            if digest != expected {
                return Err(String::from("worker runtime digest is not approved"));
            }
        }
        fcntl_add_seals(
            &snapshot,
            SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
        )
        .map_err(|_| String::from("worker runtime executable snapshot is not immutable"))?;
        Ok(ApprovedExecutable {
            image: std::sync::Arc::new(snapshot),
        })
    }

    fn create_executable_snapshot() -> Result<rustix::fd::OwnedFd, String> {
        let base_flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
        match memfd_create("ascension-verified-worker-runtime", base_flags | MemfdFlags::EXEC) {
            Ok(fd) => Ok(fd),
            Err(error) if error == rustix::io::Errno::INVAL => {
                memfd_create("ascension-verified-worker-runtime", base_flags)
                    .map_err(|_| String::from("worker runtime executable snapshot unavailable"))
            }
            Err(_) => Err(String::from(
                "worker runtime executable snapshot unavailable",
            )),
        }
    }

    fn hash_and_copy(source: &mut File, snapshot: &mut File) -> Result<String, String> {
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 32 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|_| String::from("worker executable could not be hashed"))?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(
                    u64::try_from(count)
                        .map_err(|_| String::from("worker executable size overflowed"))?,
                )
                .ok_or_else(|| String::from("worker executable size overflowed"))?;
            if total > MAX_EXECUTABLE_BYTES {
                return Err(String::from("worker executable exceeds its size bound"));
            }
            snapshot
                .write_all(&buffer[..count])
                .map_err(|_| String::from("worker executable snapshot could not be written"))?;
            hasher.update(&buffer[..count]);
        }
        Ok(crate::hex_bytes(hasher.finalize()))
    }

    fn approved_environment() -> Vec<(OsString, OsString)> {
        const DENY: &[&str] = &[
            "STS2_WORKER_ENDPOINT_NAMESPACE",
            "STS2_WORKER_CREDENTIAL_PATH",
            "STS2_WORKER_RUNTIME_BINARY",
            "STS2_WORKER_RUNTIME_SHA256",
            "STS2_WORKER_TIMEOUT_MS",
        ];
        std::env::vars_os()
            .filter(|(key, _)| {
                let text = key.to_string_lossy();
                (text.starts_with("STS2_")
                    || matches!(text.as_ref(), "PATH" | "HOME" | "TMPDIR" | "LANG"))
                    && !DENY.iter().any(|denied| *denied == text)
                    && text != "STS2_ATTEMPT_ID"
            })
            .collect()
    }

    fn read_exact_deadline(
        reader: &mut File,
        target: &mut [u8],
        deadline: Instant,
        label: &str,
    ) -> Result<(), String> {
        let mut offset = 0;
        while offset < target.len() {
            if Instant::now() >= deadline {
                return Err(format!("{label} timed out"));
            }
            match reader.read(&mut target[offset..]) {
                Ok(0) => return Err(format!("{label} is truncated")),
                Ok(count) => offset += count,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(POLL_INTERVAL);
                }
                Err(_) => return Err(format!("{label} could not be read")),
            }
        }
        Ok(())
    }
