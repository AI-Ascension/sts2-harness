// SPDX-License-Identifier: MIT

    fn exact_fields(
        object: &serde_json::Map<String, Value>,
        fields: &[&str],
    ) -> Result<(), String> {
        if object.len() != fields.len()
            || object
                .keys()
                .any(|key| !fields.iter().any(|field| *field == key))
        {
            return Err(String::from("worker bootstrap schema is not closed"));
        }
        Ok(())
    }

    fn peer_string<'a>(
        object: &'a serde_json::Map<String, Value>,
        name: &str,
    ) -> Result<&'a str, String> {
        object
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("worker bootstrap field is invalid"))
    }

    fn bounded_u32(
        object: &serde_json::Map<String, Value>,
        name: &str,
        positive: bool,
    ) -> Result<u32, String> {
        let value = object
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| String::from("worker bootstrap number is invalid"))?;
        let value = u32::try_from(value)
            .map_err(|_| String::from("worker bootstrap number is out of bounds"))?;
        if positive && value == 0 {
            return Err(String::from("worker bootstrap PID is invalid"));
        }
        Ok(value)
    }

    fn uuid4_string(object: &serde_json::Map<String, Value>, name: &str) -> Result<String, String> {
        let value = peer_string(object, name)?;
        let parsed = uuid::Uuid::parse_str(value)
            .map_err(|_| String::from("worker bootstrap UUID is invalid"))?;
        if parsed.get_version_num() != 4
            || parsed.get_variant() != uuid::Variant::RFC4122
            || parsed.to_string() != value
        {
            return Err(String::from("worker bootstrap UUID is invalid"));
        }
        Ok(value.to_owned())
    }

    fn validate_component(value: &str) -> Result<(), String> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 128
            || value == "."
            || value == ".."
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.'))
        {
            return Err(String::from("worker bootstrap component is invalid"));
        }
        Ok(())
    }

    fn validate_creation_token(value: &str) -> Result<(), String> {
        if value.is_empty()
            || value.len() > 20
            || value.starts_with('0')
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || value.parse::<u64>().ok().is_none_or(|token| token == 0)
        {
            return Err(String::from("worker bootstrap creation token is invalid"));
        }
        Ok(())
    }

    fn validate_digest(value: &str) -> Result<(), ()> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(());
        }
        Ok(())
    }

    fn validate_absolute_path(value: &str, label: &str) -> Result<(), String> {
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || !Path::new(value).is_absolute()
            || value.contains('\0')
        {
            return Err(format!("{label} path is invalid"));
        }
        Ok(())
    }

    fn validate_reference(path: &Path, label: &str) -> Result<(), String> {
        let value = path
            .to_str()
            .ok_or_else(|| format!("{label} path is not UTF-8"))?;
        validate_absolute_path(value, label)?;
        if path.file_name().is_none()
            || path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
            || ["/dev", "/proc", "/sys"]
                .iter()
                .any(|root| path.starts_with(root))
        {
            return Err(format!("{label} path is not a protected local reference"));
        }
        Ok(())
    }

    fn validate_namespace(path: &Path) -> Result<(), String> {
        let value = path
            .to_str()
            .ok_or_else(|| String::from("worker endpoint namespace is not UTF-8"))?;
        validate_absolute_path(value, "worker endpoint namespace")?;
        if value.len() > 100
            || value.ends_with('/')
            || value.contains('\\')
            || value.chars().any(char::is_control)
            || value[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(String::from("worker endpoint namespace is invalid"));
        }
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| String::from("worker endpoint namespace is unavailable"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(String::from(
                "worker endpoint namespace is not owner-protected",
            ));
        }
        Ok(())
    }

    fn derive_endpoint(namespace: &Path, nonce: &str) -> Result<PathBuf, String> {
        let endpoint = namespace.join(format!("ascension-worker-{nonce}.sock"));
        let endpoint_text = endpoint
            .to_str()
            .ok_or_else(|| String::from("worker endpoint is not UTF-8"))?;
        if endpoint_text.len() > 100 {
            return Err(String::from("worker endpoint exceeds its path bound"));
        }
        Ok(endpoint)
    }

    fn bind_endpoint(namespace: &Path, endpoint: &Path) -> Result<UnixListener, String> {
        let metadata = fs::symlink_metadata(namespace)
            .map_err(|_| String::from("worker endpoint namespace disappeared"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(String::from("worker endpoint namespace changed"));
        }
        // Binding is intentionally the first operation on the derived socket:
        // an occupied path is a hard failure and is never unlinked by name.
        let listener = UnixListener::bind(endpoint)
            .map_err(|_| String::from("worker endpoint is occupied or unavailable"))?;
        fs::set_permissions(endpoint, Permissions::from_mode(0o600))
            .map_err(|_| String::from("worker endpoint permissions are unavailable"))?;
        Ok(listener)
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FileIdentity {
        device: u64,
        inode: u64,
    }

    struct SocketGuard {
        path: PathBuf,
        identity: FileIdentity,
    }

    impl SocketGuard {
        fn new(path: PathBuf) -> Result<Self, String> {
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| String::from("worker endpoint identity is unavailable"))?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
                return Err(String::from("worker endpoint is not a Unix socket"));
            }
            Ok(Self {
                path,
                identity: FileIdentity {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                },
            })
        }
    }

    impl Drop for SocketGuard {
        fn drop(&mut self) {
            if let Ok(metadata) = fs::symlink_metadata(&self.path)
                && metadata.file_type().is_socket()
                && metadata.dev() == self.identity.device
                && metadata.ino() == self.identity.inode
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    struct PeerSession {
        _pidfd: std::os::fd::OwnedFd,
        _image: File,
        image_identity: FileIdentity,
        expected_path: PathBuf,
        expected_pid: u32,
        expected_uid: u32,
        expected_gid: u32,
        creation_token: String,
    }

    fn prove_peer_image(peer: &LinuxPeer) -> Result<PeerImageProof, String> {
        let creation_before = process_start_token(peer.pid)?;
        if creation_before != peer.creation_token {
            return Err(String::from("worker peer creation token is not approved"));
        }
        let proc_path = proc_executable(peer.pid);
        let executable = fs::read_link(&proc_path)
            .map_err(|_| String::from("worker peer executable is unavailable"))?;
        if executable != peer.executable {
            return Err(String::from("worker peer executable is not approved"));
        }
        let mut image = File::open(&proc_path)
            .map_err(|_| String::from("worker peer executable is unavailable"))?;
        let image_identity = file_identity(&image)?;
        let configured = fs::metadata(&peer.executable)
            .map_err(|_| String::from("configured worker executable is unavailable"))?;
        if configured.dev() != image_identity.device || configured.ino() != image_identity.inode {
            return Err(String::from("worker peer executable image is not approved"));
        }
        let digest = hash_file(&mut image)?;
        if digest != peer.executable_sha256 {
            return Err(String::from(
                "worker peer executable digest is not approved",
            ));
        }
        let creation_after = process_start_token(peer.pid)?;
        let path_after = fs::read_link(&proc_path)
            .map_err(|_| String::from("worker peer executable is unavailable"))?;
        if creation_after != creation_before || path_after != executable {
            return Err(String::from(
                "worker peer identity changed during authentication",
            ));
        }
        Ok(PeerImageProof {
            image: Arc::new(image),
            image_identity,
            digest,
        })
    }
