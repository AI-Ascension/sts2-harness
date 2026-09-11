// SPDX-License-Identifier: MIT

    fn write_transport_frame(
        stream: &mut UnixStream,
        body: &[u8],
        peer: &PeerSession,
    ) -> Result<(), String> {
        if body.is_empty() || body.len() > TRANSPORT_MAX_FRAME_BYTES {
            return Err(String::from("worker response exceeds its frame bound"));
        }
        let length = u32::try_from(body.len())
            .map_err(|_| String::from("worker response length is invalid"))?;
        stream
            .set_write_timeout(Some(Duration::from_millis(5_000)))
            .map_err(|error| format!("worker transport write deadline is unavailable: {error}"))?;
        peer.verify_current().map_err(|error| {
            format!("worker response could not be written: peer-before {error}")
        })?;
        stream
            .write_all(&length.to_be_bytes())
            .map_err(|error| format!("worker response could not be written: length {error}"))?;
        peer.verify_current().map_err(|error| {
            format!("worker response could not be written: peer-middle {error}")
        })?;
        stream
            .write_all(body)
            .map_err(|error| format!("worker response could not be written: body {error}"))?;
        peer.verify_current()
            .map_err(|error| format!("worker response could not be written: peer-after {error}"))
    }

    fn authenticate_connection(
        stream: &mut UnixStream,
        peer: &LinuxPeer,
        credential_path: &Path,
    ) -> Result<PeerSession, String> {
        let proof_state = spawn_peer_image_proof(peer)?;
        let session = PeerSession::authenticate(stream, peer, &proof_state)?;
        let auth = Zeroizing::new(read_transport_frame(stream, &session, AUTH_TIMEOUT)?);
        let credential = read_credential(credential_path)?;
        let mut expected = Zeroizing::new(Vec::with_capacity(AUTH_MAGIC.len() + credential.len()));
        expected.extend_from_slice(AUTH_MAGIC);
        expected.extend_from_slice(&credential);
        if !constant_time_equal(&auth, &expected) {
            return Err(String::from("worker credential is not approved"));
        }
        Ok(session)
    }

    fn authenticate_connection_owned(
        mut stream: EndpointStream,
        peer: LinuxPeer,
        credential_path: PathBuf,
    ) -> Result<AuthenticatedConnection, String> {
        let session = authenticate_connection(&mut stream, &peer, &credential_path)?;
        Ok(AuthenticatedConnection {
            stream,
            peer: session,
        })
    }

    fn owner_proof_label() -> &'static str {
        "linux-peer-and-credential-authenticated"
    }

    fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
        let mut difference = (left.len() ^ right.len()) as u8;
        let length = left.len().max(right.len());
        for index in 0..length {
            let left_byte = left.get(index).copied().unwrap_or(0);
            let right_byte = right.get(index).copied().unwrap_or(0);
            difference |= left_byte ^ right_byte;
        }
        difference == 0
    }
