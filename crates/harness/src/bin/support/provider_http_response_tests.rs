// SPDX-License-Identifier: MIT

//! Framing, status, bound, and body tests for the shared provider response reader.

use super::*;

/// Builds a `Content-Length` framed response whose declared length may deliberately disagree.
fn length_framed(status: &str, declared: usize, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Length: {declared}\r\n\r\n{body}").into_bytes()
}

#[test]
fn accepts_a_well_formed_length_framed_body() {
    let body = r#"{"answer":1}"#;
    assert_eq!(
        parse_json_response(&length_framed("200 OK", body.len(), body)).ok(),
        Some(serde_json::json!({"answer": 1}))
    );
}

#[test]
fn accepts_a_well_formed_chunked_body() {
    let response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nc\r\n{\"answer\":1}\r\n0\r\n\r\n";
    assert_eq!(
        parse_json_response(response).ok(),
        Some(serde_json::json!({"answer": 1}))
    );
}

#[test]
fn refuses_a_non_success_status() {
    let body = "{}";
    assert_eq!(
        parse_json_response(&length_framed("503 Service Unavailable", body.len(), body)).err(),
        Some(ProviderResponseError::Status)
    );
    assert_eq!(
        parse_json_response(&length_framed("429 Too Many Requests", body.len(), body)).err(),
        Some(ProviderResponseError::Status)
    );
}

#[test]
fn refuses_a_header_block_without_a_terminator() {
    assert_eq!(
        parse_json_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n").err(),
        Some(ProviderResponseError::MissingHeaders)
    );
}

#[test]
fn refuses_a_body_that_disagrees_with_its_declared_length() {
    assert_eq!(
        parse_json_response(&length_framed("200 OK", 2, r#"{"answer":1}"#)).err(),
        Some(ProviderResponseError::InvalidLength)
    );
    assert_eq!(
        parse_json_response(&length_framed("200 OK", 64, "{}")).err(),
        Some(ProviderResponseError::InvalidLength)
    );
}

#[test]
fn refuses_an_absent_duplicate_or_ambiguous_framing() {
    assert_eq!(
        parse_json_response(b"HTTP/1.1 200 OK\r\nServer: test\r\n\r\n{}").err(),
        Some(ProviderResponseError::InvalidLength)
    );
    assert_eq!(
        parse_json_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}")
            .err(),
        Some(ProviderResponseError::DuplicateLength)
    );
    assert_eq!(
        parse_json_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n{}"
        )
        .err(),
        Some(ProviderResponseError::AmbiguousFraming)
    );
    assert_eq!(
        parse_json_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n{}").err(),
        Some(ProviderResponseError::InvalidTransferEncoding)
    );
}

#[test]
fn refuses_a_header_line_without_a_separator() {
    assert_eq!(
        parse_json_response(b"HTTP/1.1 200 OK\r\nmalformed\r\n\r\n{}").err(),
        Some(ProviderResponseError::InvalidHeader)
    );
}

#[test]
fn refuses_a_body_that_is_not_json() {
    let body = "not json";
    assert_eq!(
        parse_json_response(&length_framed("200 OK", body.len(), body)).err(),
        Some(ProviderResponseError::InvalidJson)
    );
}

#[test]
fn validates_chunked_framing_without_accepting_ambiguity() {
    assert_eq!(
        decode_chunks(b"2\r\n{}\r\n0\r\n\r\n").ok(),
        Some(b"{}".to_vec())
    );
    assert_eq!(
        decode_chunks(b"3\r\n{}\r\n0\r\n\r\n").err(),
        Some(ProviderResponseError::InvalidChunkLength)
    );
    assert_eq!(
        decode_chunks(b"0\r\n\r\nextra").err(),
        Some(ProviderResponseError::UnsupportedTrailers)
    );
    assert_eq!(
        decode_chunks(b"fffffff\r\n").err(),
        Some(ProviderResponseError::InvalidChunkLength)
    );
    assert_eq!(
        decode_chunks(b"fffffffffffffffffff\r\n").err(),
        Some(ProviderResponseError::InvalidChunkSize)
    );
    assert_eq!(
        decode_chunks(b"zz\r\n{}\r\n0\r\n\r\n").err(),
        Some(ProviderResponseError::InvalidChunkSize)
    );
}
