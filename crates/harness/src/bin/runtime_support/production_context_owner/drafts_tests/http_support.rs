// SPDX-License-Identifier: MIT

fn authenticator() -> Arc<dyn Authenticator> {
    let mut authenticator = StaticAuthenticator::new();
    for (token, scopes, subject) in [
        (
            "edit-token",
            vec!["workflow:context:edit"],
            SUBJECT,
        ),
        (
            "objective-token",
            vec!["workflow:context:objective:edit"],
            SUBJECT,
        ),
        ("read-token", vec!["workflow:read"], SUBJECT),
        (
            "content-token",
            vec!["workflow:read", "workflow:context:content:read"],
            SUBJECT,
        ),
        (
            "foreign-edit-token",
            vec!["workflow:context:edit"],
            "different-actor",
        ),
    ] {
        authenticator = authenticator
            .with_credential(
                token,
                AuthContext::new(subject, scopes.into_iter().map(str::to_owned))
                    .expect("test authentication context"),
            )
            .expect("add test credential");
    }
    Arc::new(authenticator)
}

fn service(owner: Arc<Owner>, store: Arc<MemoryWorkflowStore>) -> Arc<ManagementService> {
    Arc::new(
        ManagementService::new(store)
            .with_context_owner_port(owner as Arc<dyn ContextOwnerPort>),
    )
}

fn call(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, serde_json::Value) {
    let config = ServerConfig::new("127.0.0.1:0".parse().expect("loopback"), authenticator)
        .expect("server config");
    let server = ManagementServer::start(config, service).expect("served management owner starts");
    let mut stream = TcpStream::connect(server.address()).expect("connect to served owner");
    let payload = body.unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    )
    .expect("write served request headers");
    stream.write_all(payload).expect("write served request body");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read served response");
    server.shutdown().expect("stop served owner");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response header terminator");
    let headers = std::str::from_utf8(&response[..split]).expect("HTTP response headers");
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .expect("HTTP status");
    let body: serde_json::Value = serde_json::from_slice(&response[split + 4..])
        .expect("served JSON response");
    (status, body)
}

fn receipt(value: &serde_json::Value) -> ContextOwnerMutationReceipt {
    serde_json::from_value(value.clone()).expect("mutation receipt")
}

fn error_code(value: &serde_json::Value) -> &str {
    value["error"]["code"]
        .as_str()
        .expect("management error code")
}

fn assert_storage_files_hide(directory: &PathBuf, secret: &str) {
    let secret = secret.as_bytes();
    let files = fs::read_dir(directory)
        .expect("read encrypted owner-store directory")
        .map(|entry| entry.expect("read owner-store entry").path())
        .collect::<Vec<_>>();
    assert!(!files.is_empty(), "owner store must persist files");
    for path in files {
        let bytes = fs::read(&path).expect("read SQLite database or WAL file");
        assert!(
            !bytes.windows(secret.len()).any(|window| window == secret),
            "authored plaintext appeared in owner database or WAL file"
        );
    }
}

fn run_path(run_id: &str, suffix: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/{suffix}")
}
