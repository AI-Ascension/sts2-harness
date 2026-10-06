// SPDX-License-Identifier: MIT

fn authenticator() -> Arc<dyn Authenticator> {
    let mut authenticator = StaticAuthenticator::new();
    for (token, scopes, subject) in [
        ("edit-token", vec!["workflow:context:edit"], SUBJECT),
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
            "publish-token",
            vec![
                "workflow:read",
                "workflow:control",
                "workflow:content:write",
            ],
            SUBJECT,
        ),
        (
            "foreign-edit-token",
            vec!["workflow:context:edit"],
            "different-actor",
        ),
        (
            "foreign-read-token",
            vec!["workflow:read"],
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
        ManagementService::new(store).with_context_owner_port(owner as Arc<dyn ContextOwnerPort>),
    )
}

struct HttpCallOptions<'a> {
    content_type: Option<&'a str>,
    max_body_bytes: Option<usize>,
}

fn call(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, serde_json::Value) {
    call_with_options(
        service,
        authenticator,
        token,
        method,
        path,
        body,
        HttpCallOptions {
            content_type: Some("application/json"),
            max_body_bytes: None,
        },
    )
}

fn call_with_options(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    options: HttpCallOptions<'_>,
) -> (u16, serde_json::Value) {
    let mut config = ServerConfig::new("127.0.0.1:0".parse().expect("loopback"), authenticator)
        .expect("server config");
    if let Some(max_body_bytes) = options.max_body_bytes {
        config.limits.max_body_bytes = max_body_bytes;
    }
    let server = ManagementServer::start(config, service).expect("served management owner starts");
    let mut stream = TcpStream::connect(server.address()).expect("connect to served owner");
    let payload = body.unwrap_or_default();
    let content_type_header = options
        .content_type
        .map_or_else(String::new, |value| format!("Content-Type: {value}\r\n"));
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n{content_type_header}Connection: close\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    request.extend_from_slice(payload);
    stream.write_all(&request).expect("write served request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read served response");
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
    let body: serde_json::Value =
        serde_json::from_slice(&response[split + 4..]).expect("served JSON response");
    (status, body)
}

fn client_call(
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
    let response = ManagementClient::new(server.address(), token)
        .expect("management client")
        .request_json(method, path, body)
        .expect("management client receives served response");
    server.shutdown().expect("stop served owner");
    let body = serde_json::from_slice(&response.body).expect("served JSON response");
    (response.status, body)
}

fn lookup_mutation_receipt(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    run_id: &str,
    request: ContextOwnerMutationRequest,
) -> (u16, serde_json::Value) {
    let lookup = ContextOwnerMutationLookupRequest {
        schema_version: CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION.to_owned(),
        request,
    };
    let body = serde_json::to_vec(&lookup).expect("mutation receipt lookup JSON");
    call(
        service,
        authenticator,
        token,
        "POST",
        &run_path(run_id, "context-owner-mutation-receipts/lookup"),
        Some(&body),
    )
}

fn current_drafts(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    run_id: &str,
) -> serde_json::Value {
    let (status, drafts) = call(
        service,
        authenticator,
        "read-token",
        "GET",
        &run_path(run_id, "context-owner-drafts"),
        None,
    );
    assert_eq!(status, 200, "read served owner drafts");
    drafts
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
