// SPDX-License-Identifier: MIT

    impl PeerSession {
        fn authenticate(
            stream: &UnixStream,
            peer: &LinuxPeer,
            proof_state: &PeerProofState,
        ) -> Result<Self, String> {
            set_socket_passcred(stream, true)
                .map_err(|_| String::from("worker peer message credentials are unavailable"))?;
            let credentials = socket_peercred(stream)
                .map_err(|_| String::from("worker peer credentials are unavailable"))?;
            let actual_pid = u32::try_from(credentials.pid.as_raw_pid())
                .map_err(|_| String::from("worker peer PID is invalid"))?;
            if actual_pid != peer.pid
                || credentials.uid.as_raw() != peer.uid
                || credentials.gid.as_raw() != peer.gid
            {
                return Err(String::from("worker peer credentials are not approved"));
            }
            let pid = Pid::from_raw(
                i32::try_from(peer.pid).map_err(|_| String::from("worker peer PID is invalid"))?,
            )
            .ok_or_else(|| String::from("worker peer PID is zero"))?;
            let pidfd = pidfd_open(pid, PidfdFlags::empty())
                .map_err(|_| String::from("worker peer process identity is unavailable"))?;
            let creation_token = process_start_token(peer.pid)?;
            if creation_token != peer.creation_token {
                return Err(String::from("worker peer creation token is not approved"));
            }
            let proc_path = proc_executable(peer.pid);
            let executable = fs::read_link(&proc_path)
                .map_err(|_| String::from("worker peer executable is unavailable"))?;
            if executable != peer.executable {
                return Err(String::from("worker peer executable is not approved"));
            }
            let proof = proof_state.wait()?;
            if proof.digest != peer.executable_sha256 {
                return Err(String::from(
                    "worker peer executable digest is not approved",
                ));
            }
            let image = File::open(&proc_path)
                .map_err(|_| String::from("worker peer executable is unavailable"))?;
            let image_identity = file_identity(&image)?;
            let configured = fs::metadata(&peer.executable)
                .map_err(|_| String::from("configured worker executable is unavailable"))?;
            if configured.dev() != image_identity.device || configured.ino() != image_identity.inode
            {
                return Err(String::from("worker peer executable image is not approved"));
            }
            if image_identity != proof.image_identity {
                return Err(String::from("worker peer executable image changed"));
            }
            let held_image = proof
                .image
                .try_clone()
                .map_err(|_| String::from("worker peer executable is unavailable"))?;
            let start_after = process_start_token(peer.pid)?;
            let path_after = fs::read_link(&proc_path)
                .map_err(|_| String::from("worker peer executable is unavailable"))?;
            if start_after != creation_token || path_after != executable {
                return Err(String::from(
                    "worker peer identity changed during authentication",
                ));
            }
            Ok(Self {
                _pidfd: pidfd,
                _image: held_image,
                image_identity,
                expected_path: executable,
                expected_pid: peer.pid,
                expected_uid: peer.uid,
                expected_gid: peer.gid,
                creation_token,
            })
        }

        fn verify_current(&self) -> Result<(), String> {
            if process_start_token(self.expected_pid)? != self.creation_token {
                return Err(String::from("worker peer creation token changed"));
            }
            let proc_path = proc_executable(self.expected_pid);
            let path = fs::read_link(&proc_path)
                .map_err(|_| String::from("worker peer executable disappeared"))?;
            if path != self.expected_path {
                return Err(String::from("worker peer executable path changed"));
            }
            let image = File::open(&proc_path)
                .map_err(|_| String::from("worker peer executable disappeared"))?;
            if file_identity(&image)? != self.image_identity {
                return Err(String::from("worker peer executable image changed"));
            }
            Ok(())
        }

        fn verify_message_credentials(
            &self,
            credentials: &rustix::net::UCred,
        ) -> Result<(), String> {
            let pid = u32::try_from(credentials.pid.as_raw_pid())
                .map_err(|_| String::from("worker peer message PID is invalid"))?;
            if pid != self.expected_pid
                || credentials.uid.as_raw() != self.expected_uid
                || credentials.gid.as_raw() != self.expected_gid
            {
                return Err(String::from("worker peer message credentials changed"));
            }
            Ok(())
        }
    }

    fn proc_executable(pid: u32) -> PathBuf {
        PathBuf::from(format!("/proc/{pid}/exe"))
    }

    fn process_start_token(pid: u32) -> Result<String, String> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|_| String::from("worker peer process start token is unavailable"))?;
        let close = stat
            .rfind(')')
            .ok_or_else(|| String::from("worker peer process start token is malformed"))?;
        stat[close + 2..]
            .split_whitespace()
            .nth(19)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| String::from("worker peer process start token is unavailable"))
    }

    fn file_identity(file: &File) -> Result<FileIdentity, String> {
        let metadata = fstat(file)
            .map_err(|_| String::from("worker executable file identity is unavailable"))?;
        Ok(FileIdentity {
            device: metadata.st_dev,
            inode: metadata.st_ino,
        })
    }

    fn hash_file(file: &mut File) -> Result<String, String> {
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 32 * 1024];
        loop {
            let count = file
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
            hasher.update(&buffer[..count]);
        }
        Ok(crate::hex_bytes(hasher.finalize()))
    }

    fn read_credential(path: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
        validate_reference(path, "worker credential")?;
        let descriptor = open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| String::from("worker credential is unavailable"))?;
        let mut file: File = descriptor.into();
        let metadata =
            fstat(&file).map_err(|_| String::from("worker credential is unavailable"))?;
        let mode = metadata.st_mode;
        if (mode & 0o170_000) != 0o100_000
            || metadata.st_uid != geteuid().as_raw()
            || mode & 0o077 != 0
            || metadata.st_size <= 0
            || u64::try_from(metadata.st_size)
                .ok()
                .is_none_or(|size| size > MAX_CREDENTIAL_BYTES as u64)
        {
            return Err(String::from("worker credential is not owner-only"));
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_CREDENTIAL_BYTES));
        let mut buffer = [0_u8; 256];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| String::from("worker credential could not be read"))?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_CREDENTIAL_BYTES {
                return Err(String::from("worker credential is oversized"));
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        if bytes.is_empty() {
            return Err(String::from("worker credential is empty"));
        }
        Ok(bytes)
    }

    fn recv_message(
        stream: &mut UnixStream,
        buffer: &mut [u8],
        peer: &PeerSession,
    ) -> Result<usize, String> {
        use std::mem::MaybeUninit;
        let mut control_space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1), ScmCredentials(1))];
        let mut control = rustix::net::RecvAncillaryBuffer::new(&mut control_space);
        let mut iov = [IoSliceMut::new(buffer)];
        let message = rustix::net::recvmsg(
            &mut *stream,
            &mut iov,
            &mut control,
            rustix::net::RecvFlags::CMSG_CLOEXEC,
        )
        .map_err(|error| {
            if matches!(
                error,
                rustix::io::Errno::TIMEDOUT | rustix::io::Errno::WOULDBLOCK
            ) {
                String::from("worker transport deadline expired")
            } else {
                String::from("worker transport read failed")
            }
        })?;
        if message.bytes == 0 {
            return Err(String::from("worker transport closed during read"));
        }
        if message.flags.contains(rustix::net::ReturnFlags::CTRUNC) {
            return Err(String::from(
                "worker transport ancillary data was truncated",
            ));
        }
        let mut credentials = None;
        for ancillary in control.drain() {
            match ancillary {
                RecvAncillaryMessage::ScmCredentials(value) => {
                    if credentials.replace(value).is_some() {
                        return Err(String::from(
                            "worker transport carried duplicate credentials",
                        ));
                    }
                }
                RecvAncillaryMessage::ScmRights(_) => {
                    return Err(String::from("worker transport carried file descriptors"));
                }
                _ => {
                    return Err(String::from(
                        "worker transport carried unsupported metadata",
                    ));
                }
            }
        }
        peer.verify_message_credentials(
            &credentials.ok_or_else(|| String::from("worker transport omitted credentials"))?,
        )?;
        Ok(message.bytes)
    }

    fn read_transport_frame(
        stream: &mut UnixStream,
        peer: &PeerSession,
    ) -> Result<Vec<u8>, String> {
        let timeout = Duration::from_millis(5_000);
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| String::from("worker transport read deadline is unavailable"))?;
        let mut length_bytes = [0_u8; 4];
        read_message_exact(stream, &mut length_bytes, peer)?;
        let length = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| String::from("worker transport frame length is invalid"))?;
        if length == 0 || length > TRANSPORT_MAX_FRAME_BYTES {
            return Err(String::from("worker transport frame exceeds its bound"));
        }
        let mut body = vec![0_u8; length];
        read_message_exact(stream, &mut body, peer)?;
        Ok(body)
    }

    fn read_message_exact(
        stream: &mut UnixStream,
        target: &mut [u8],
        peer: &PeerSession,
    ) -> Result<(), String> {
        let mut offset = 0;
        while offset < target.len() {
            peer.verify_current()?;
            let count = recv_message(stream, &mut target[offset..], peer)?;
            offset += count;
            peer.verify_current()?;
        }
        Ok(())
    }
