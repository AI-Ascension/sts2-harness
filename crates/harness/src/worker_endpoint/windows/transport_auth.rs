// SPDX-License-Identifier: MIT

use zeroize::Zeroizing;

struct PeerSession;

fn authenticate_connection_owned(
    mut stream: EndpointStream,
    peer: WindowsPeer,
    credential_path: PathBuf,
) -> Result<AuthenticatedConnection, String> {
    let session = authenticate_connection(&mut stream, &peer, &credential_path)?;
    Ok(AuthenticatedConnection {
        stream,
        peer: session,
    })
}

fn authenticate_connection(
    stream: &mut EndpointStream,
    _peer: &WindowsPeer,
    credential_path: &std::path::Path,
) -> Result<PeerSession, String> {
    let auth = Zeroizing::new(read_transport_frame(stream, &PeerSession, AUTH_TIMEOUT)?);
    let credential = sts2_harness_windows_boundary::read_protected_credential(
        credential_path,
        MAX_CREDENTIAL_BYTES,
    )?;
    let mut expected = Zeroizing::new(Vec::with_capacity(AUTH_MAGIC.len() + credential.len()));
    expected.extend_from_slice(AUTH_MAGIC);
    expected.extend_from_slice(&credential);
    if !constant_time_equal(&auth, &expected) {
        return Err(String::from("worker credential is not approved"));
    }
    Ok(PeerSession)
}

fn read_transport_frame(
    stream: &mut EndpointStream,
    _peer: &PeerSession,
    timeout: std::time::Duration,
) -> Result<Vec<u8>, String> {
    stream.read_frame(timeout, TRANSPORT_MAX_FRAME_BYTES)
}

fn write_transport_frame(
    stream: &mut EndpointStream,
    body: &[u8],
    _peer: &PeerSession,
) -> Result<(), String> {
    stream.write_frame(body, TRANSPORT_TIMEOUT, TRANSPORT_MAX_FRAME_BYTES)
}

fn owner_proof_label() -> &'static str {
    "windows-pipe-and-credential-authenticated"
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
