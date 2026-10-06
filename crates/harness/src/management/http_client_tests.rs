// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

// The contract under test is the exact byte sequence this client writes to the
// socket, because a server rejects any header outside its allow-list by name.
// Asserting on the constructed head is therefore the only assertion that can
// catch this class of defect: a request that is "logically the same" but carries
// one extra header is refused in production and accepted here.
//
// ## Which server, and why only one of them has an allow-list
//
// The invariant enforced below is **"every header this client sends is admitted
// by the server it is pointed at."** That server is today the harness management
// listener, not the gateway. Only the gateway enforces a header allow-list;
// `Idempotency-Key` is the case that makes the difference visible:
//
// * It is **required** by the harness management server -- `http_routes_memory_owner.rs`
//   returns `idempotency_key_required` when it is absent, and 16 operations across
//   `memory-api` (10) and `control-api` (6) declare it `required: true` in their
//   OpenAPI contracts.
// * It is **absent** from the gateway's `header_is_allowed`, which has no exemption
//   clause at all.
//
// The harness management server runs no allow-list of its own, so there is nothing to
// assert the rest of the keyed head against; only the requirement above is pinned. A
// hand-written "management allow-list" would be fiction, and treating fiction as a
// server contract is the mistake #598 recorded.
//
// So the header must be sent, and it must never be sent to the gateway. These tests are
// pinned to the harness listener's requirements; a reader must not conclude from this file
// alone that the header is safe to send to the gateway. If a caller is ever pointed at the
// gateway, that caller is the defect -- it is not this client's job to silently drop a header
// the target server requires.
//
// Issue #560 removed `accept`. Issue #598 removed a fabricated `idempotency-key` entry from
// the allow-list below and made the idempotent path actually asserted: the previous version
// asserted only over a head built with `None`, so the entry could neither fail nor be reached.

/// The gateway's allow-list, transcribed from `header_is_allowed` in sts2-gateway
/// (`crates/gateway/src/bin/runtime_support/service_authorization.rs`). Keep it in step with
/// that function.
///
/// One const, one consumer. When this list was declared separately inside each test, the
/// copies could disagree and the guard read the copy nothing checked -- inserting
/// `idempotency-key` into the enforcing copy left the whole suite green, which is the
/// defect #598 exists to remove. Hoisting is what makes a transcription error detectable:
/// `the_transcribed_allow_list_matches_the_gateway` reads this exact const.
const GATEWAY_ALLOW_LIST: &[&str] = &[
    "authorization",
    "connection",
    "content-length",
    "content-type",
    "host",
    "x-mcp-request-id",
    "x-mcp-session-id",
    "x-sts2-instance-id",
    "x-sts2-caller-id",
    "x-sts2-session-id",
    "x-sts2-lease-id",
    "x-sts2-lease-epoch",
    "x-sts2-workflow-boot-epoch",
    "x-sts2-correlation-id",
    "x-sts2-capabilities-version",
    "x-sts2-episode-profile",
    "x-sts2-peer-token",
    "x-sts2-recovery-capability",
];

/// A plain head: no idempotency key, so this is the *minimum* header set.
fn head() -> String {
    request_head(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
        "POST",
        "/v1/workflows",
        None,
        2,
    )
}

/// The same request carrying an idempotency key. Every caller of
/// `request_json_with_idempotency_key` takes this path, so it is asserted
/// alongside the plain one rather than assumed to be a subset of it.
fn idempotent_head() -> String {
    request_head(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
        "POST",
        "/v1/policy/proposals",
        Some("key-42"),
        2,
    )
}

fn header_names(head: &str) -> Vec<String> {
    head.split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim().to_ascii_lowercase())
        .collect()
}

#[test]
fn the_request_head_carries_no_accept_header() {
    let names = header_names(&head());
    assert!(
        !names.iter().any(|name| name == "accept"),
        "accept must not reach the gateway: it is absent from the allow-list, \
         which refuses it by name. Sent headers were {names:?}"
    );
}

#[test]
fn every_sent_header_is_one_the_gateway_allow_list_admits() {
    // The gateway's allow-list, transcribed exactly -- no more, no fewer. Keep this in step
    // with `header_is_allowed` in sts2-gateway. A name added to the client without being
    // admitted there is the defect #560 was; a name added to *this* list without being
    // admitted there converts a real refusal into a green build, which is worse than no test.
    // The PLAIN head must satisfy the gateway list outright. This is the request
    // that could be pointed at the gateway with no rework.
    let plain_names = header_names(&head());
    assert!(
        plain_names
            .iter()
            .all(|name| GATEWAY_ALLOW_LIST.contains(&name.as_str())),
        "the plain head sends a header outside the gateway allow-list, which would be \
         refused with 400 unsupported_header; sent headers were {plain_names:?}"
    );

    // The IDEMPOTENT head is asserted against the hop it is actually for. It sends
    // `idempotency-key`, which the gateway refuses, so asserting it against this
    // list would be asserting something false. It is instead checked against the
    // management server's requirement below.
    let idempotent_names = header_names(&idempotent_head());
    let refused_by_gateway: Vec<&String> = idempotent_names
        .iter()
        .filter(|name| !GATEWAY_ALLOW_LIST.contains(&name.as_str()))
        .collect();
    assert_eq!(
        refused_by_gateway,
        vec![&"idempotency-key".to_owned()],
        "the idempotent head must differ from the gateway list by exactly the one header \
         that separates the two hops; anything else is a new unadmitted header: {idempotent_names:?}"
    );
}

/// Pins the transcription itself, which is the half a behavioural test cannot reach.
///
/// `every_sent_header_is_one_the_gateway_allow_list_admits` proves the client sends nothing
/// outside this list. It cannot prove the list matches the gateway, because a name the
/// gateway never had would be admitted by the test just as readily as a real one -- and a
/// name added here silently converts a real refusal into a green build.
///
/// This is deliberately the weaker claim, not a cross-repo one: reading the gateway's
/// `header_is_allowed` at test time would make a unit test depend on a sibling repository's
/// checkout. Asserting the exact name *set* makes a gateway-side change visible as a deliberate
/// edit here rather than as a silent divergence, and the independent re-derivation against the
/// real `header_is_allowed` is the reviewer's job.
#[test]
fn the_transcribed_allow_list_matches_the_gateway() {
    // Asserted by name, not by count. A count plus a few negative names cannot see a
    // *substitution*: replace one real name with a fabricated one, keep the length at
    // 18, and the count still matches while none of the negatives moved. That is the
    // hole #613 exists to close -- #608's reviewer explicitly declined to sign off the
    // 18 names on a count pin alone, for exactly this reason. Every name is named
    // here, so a swap fails on the name that is no longer present.
    const EXPECTED: &[&str] = &[
        "authorization",
        "connection",
        "content-length",
        "content-type",
        "host",
        "x-mcp-request-id",
        "x-mcp-session-id",
        "x-sts2-instance-id",
        "x-sts2-caller-id",
        "x-sts2-session-id",
        "x-sts2-lease-id",
        "x-sts2-lease-epoch",
        "x-sts2-workflow-boot-epoch",
        "x-sts2-correlation-id",
        "x-sts2-capabilities-version",
        "x-sts2-episode-profile",
        "x-sts2-peer-token",
        "x-sts2-recovery-capability",
    ];
    assert_eq!(
        GATEWAY_ALLOW_LIST, EXPECTED,
        "header_is_allowed in sts2-gateway \
         (crates/gateway/src/bin/runtime_support/service_authorization.rs) admits a different \
         name set than this transcription; a count and a few negatives cannot see a \
         same-count substitution, so every name is compared"
    );
    for name in ["accept", "idempotency-key", "user-agent", "expect"] {
        assert!(
            !GATEWAY_ALLOW_LIST.contains(&name),
            "{name:?} is asserted as admitted by the gateway but is not in \
             header_is_allowed"
        );
    }
}

#[test]
fn patch_client_requests_require_a_json_body() {
    let client = ManagementClient::new(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
    )
    .expect("management client accepts a loopback address");
    let error = client
        .request_json(
            "PATCH",
            "/v1/workflow-runs/run/context-owner-drafts/draft",
            None,
        )
        .expect_err("PATCH requests require a JSON body before connecting");
    assert_eq!(error.code, "body_required");
}

/// **Non-vacuity.** The guard above compares two consts, so a linter or a future
/// edit could make the expected list track the actual one and the assertion would
/// pass on a corrupted transcription. This is the substitution from #613 measured
/// directly: swap one real gateway name for a fabricated one, keeping the length
/// unchanged, and confirm the shape the old guard could not see now fails.
#[test]
fn a_same_count_substitution_is_caught() {
    // The pre-#613 guard, transcribed: length plus the four negatives.
    fn pre_fix_guard(list: &[&str]) -> bool {
        list.len() == 18
            && !["accept", "idempotency-key", "user-agent", "expect"]
                .iter()
                .any(|name| list.contains(name))
    }
    assert!(
        pre_fix_guard(GATEWAY_ALLOW_LIST),
        "the real transcription must satisfy the pre-fix guard, or this control \
         is not measuring the substitution it claims to"
    );

    // Exactly the defect #613 reports: a real name replaced by a fabricated one,
    // length unchanged, none of the four negatives touched.
    let mut substituted = GATEWAY_ALLOW_LIST.to_vec();
    let index = substituted
        .iter()
        .position(|name| *name == "x-sts2-peer-token")
        .expect("the transcription contains the name the issue substitutes");
    substituted[index] = "x-sts2-fabricated-header";
    assert_eq!(
        substituted.len(),
        GATEWAY_ALLOW_LIST.len(),
        "the substitution must preserve the count, or it is not the defect in question"
    );
    assert!(
        pre_fix_guard(&substituted),
        "the pre-fix guard is expected to ACCEPT this substitution -- that is the \
         defect. If it now rejects, this control no longer measures what #613 reports."
    );
    assert_ne!(
        GATEWAY_ALLOW_LIST,
        substituted.as_slice(),
        "the name-set guard must reject a same-count substitution, which is the \
         whole of #613"
    );
}

/// The idempotent head is not a gateway request, so the gateway's allow-list does not
/// govern it. This pins the half that *is* real: the harness management server
/// **requires** `idempotency-key`, so the client must still send it.
///
/// There is deliberately no `MANAGEMENT_ALLOWED` list to check the rest of the head
/// against. The harness management server runs no header allow-list at all — it has
/// no `header_is_allowed` equivalent and never returns `unsupported_header`; the only
/// header it inspects is `idempotency-key`, in
/// `http_routes_memory_owner.rs::idempotency_key`, which requires it to be present,
/// non-empty, and at most 128 bytes. A hand-written list of names it "admits" would
/// assert an enforcement rule that does not exist, which is the same mistake #598
/// recorded: inventing a list and then treating the list as the server's contract.
///
/// Pinning only the requirement is what stops someone "fixing" the allow-list test
/// by deleting the header, which would turn 16 contract-required policy mutations
/// into 400s.
#[test]
fn the_idempotent_key_the_management_server_requires_is_still_sent() {
    let names = header_names(&idempotent_head());
    assert!(
        names.iter().any(|name| name == "idempotency-key"),
        "policy mutations require this header; without it the server returns \
         idempotency_key_required: {names:?}"
    );
}

#[test]
fn the_required_headers_and_the_body_length_are_still_present() {
    let head = head();
    assert!(
        head.starts_with("POST /v1/workflows HTTP/1.1\r\n"),
        "{head}"
    );
    assert!(head.contains("\r\nHost: 127.0.0.1:1\r\n"), "{head}");
    assert!(
        head.contains("\r\nAuthorization: Bearer test-token\r\n"),
        "{head}"
    );
    assert!(
        head.contains("\r\nContent-Type: application/json\r\n"),
        "{head}"
    );
    assert!(head.contains("\r\nContent-Length: 2\r\n"), "{head}");
    assert!(head.contains("\r\nConnection: close\r\n"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "{head}");
}

#[test]
fn an_idempotency_key_is_still_emitted_when_one_is_supplied() {
    let head = idempotent_head();
    assert!(head.contains("\r\nIdempotency-Key: key-42\r\n"), "{head}");
    // The optional segment must not disturb the rest of the head.
    assert!(head.contains("\r\nContent-Length: 2\r\n"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "{head}");
}

/// The guard above is only worth having if it can go red. This test pins that property
/// directly, because its absence is exactly what let #598 exist: with `idempotency-key` in
/// the list, the guard happily passed over a head the gateway refuses.
#[test]
fn the_allow_list_guard_is_not_vacuous() {
    // The contested header is real, is sent, and is NOT admitted by the gateway. If the
    // idempotent head ever stopped carrying it, the guard above would be asserting over a
    // request that does not exist.
    let names = header_names(&idempotent_head());
    assert!(
        names.iter().any(|name| name == "idempotency-key"),
        "the idempotent path must actually carry the header: {names:?}"
    );
    assert!(
        !GATEWAY_ALLOW_LIST.contains(&"idempotency-key"),
        "the gateway does not admit idempotency-key; if this becomes true the two hops have \
         converged and the module comment must be revisited"
    );
    assert!(
        !names
            .iter()
            .all(|name| GATEWAY_ALLOW_LIST.contains(&name.as_str())),
        "a head the gateway refuses must not pass an allow-list guard, or the guard proves \
         nothing about the header it exists to police"
    );

    // And a header in neither list is caught: the mutation this guard must reject.
    let mutated = head().replace(
        "Content-Type: application/json",
        "Content-Type: application/json\r\nX-Not-Admitted: 1",
    );
    assert!(
        !header_names(&mutated)
            .iter()
            .all(|name| GATEWAY_ALLOW_LIST.contains(&name.as_str())),
        "an unadmitted header must fail the guard"
    );
}
