// SPDX-License-Identifier: MIT

//! Serving one management connection with request-lifecycle reporting.
//!
//! Split out of `http.rs` so the accept loop, the connection worker and the reporting
//! contract stay separately reviewable, and so this file can carry the reasoning for
//! *why* each marker is emitted where it is without burying the server's structure.
//!
//! The contract, for harness
//! [#820](https://github.com/AI-Ascension/sts2-harness/issues/820), is one sentence:
//!
//! **A connection emits exactly one terminal line, and every terminal line is
//! attributable -- either it follows a start line that named the request, or it is
//! itself named. The absence of a terminal line is the signal that the connection did
//! not finish.**
//!
//! Three consequences shape the code below.
//!
//! - The start marker is emitted *after* the request is parsed, never before. Until
//!   the read succeeds there is no method and no route, and a start line that named a
//!   route the server never received would be a lie in the one log that is supposed to
//!   be trustworthy.
//! - Exactly one terminal marker is emitted, and which one depends on whether the
//!   response was *transmitted*, not on whether it was *computed*. A response that was
//!   computed and delivered is `request_end` even when it is a 4xx; a response that was
//!   computed and could not be delivered is `request_abandoned`.
//! - A connection that ended without a readable request cannot have a start marker, so
//!   its terminal marker is `request_unreadable` -- a marker that names its own
//!   condition instead of borrowing a `request_end` it has no start line for. A
//!   terminal line with no start line and no route would be unattributable, which is
//!   the defect class harness #819 documents for the pre-existing `eprintln!`.
//! - The terminal marker is emitted on every path out of this function. A marker that
//! - The terminal marker is emitted on every path out of this function. A marker that
//!   can be skipped by an early return would recreate exactly the silence this work
//!   exists to remove. A *panic* would have done the same, so `dispatch` is unwound:
//!   a panicking handler is reported as `request_panicked` and the panic is then
//!   resumed, rather than being swallowed or being allowed to masquerade as a stall.
//!   Without that, a start line with no terminal line would read identically for a
//!   hang and for a crash, which is the one distinction this log exists to make.
//!
//! # Coordination with harness #819
//!
//! #819 (PR #821) reports an undeliverable response through a `ManagementFailureSink`
//! port that names the peer, the route and the cause. This function deliberately keeps
//! that return contract: on failure it returns the route alongside the error, and the
//! caller reports it through #819's port. The two therefore describe one event at two
//! resolutions -- #819 at the connection, #820 per request -- and neither introduces a
//! second report mechanism for the same failure.

use std::net::TcpStream;
use std::panic::AssertUnwindSafe;
use std::time::Instant;

use super::lifecycle_log::RequestLifecycleLog;
use super::parse::read_request;
use super::response::write_response;
use super::routes::dispatch;
use super::*;
use crate::management::auth::Authenticator;
use crate::management::service::ManagementService;

/// Serve one connection and report its lifecycle to `lifecycle_log`.
///
/// The error arm returns the route for #819's failure port, exactly as its
/// `handle_connection` did, so this function is a drop-in for it: the caller sees the
/// same value, and the lifecycle markers are emitted in addition.
pub(super) fn serve_reported(
    mut stream: TcpStream,
    service: &ManagementService,
    authenticator: &dyn Authenticator,
    limits: &HttpLimits,
    lifecycle_log: &RequestLifecycleLog,
) -> Result<(), (Option<String>, HttpError)> {
    // Opened before the first read so the record exists even if the peer never
    // finishes a request. It emits nothing on its own.
    let lifecycle = lifecycle_log.begin();
    // Each phase gets its own budget rather than sharing one taken before the read.
    // A shared deadline meant an overran read left the write with an expired budget,
    // which `write_with_deadline` refuses *before* any syscall: the peer got nothing
    // instead of the computed response, and saw a silent close (`ECONNRESET`) for a
    // request the owner had answered. That is what made Studio's
    // `submission_refused_502` undiagnosable (#816, studio #214). The bound itself is
    // unchanged -- `limits.deadline` still caps each phase.
    let read_deadline = Instant::now() + limits.deadline;
    // The route is captured before dispatch consumes the request, so #819's failure
    // report can name it. A request that never parsed has no route and is reported
    // without one, rather than with a guessed one.
    // `unreadable_code` is the type code of a request that never parsed. It is the
    // only case in which a terminal marker may be emitted with no start marker before
    // it, and it is read by the terminal-marker match below.
    let mut unreadable_code: Option<String> = None;
    let (route, response) = match read_request(&mut stream, read_deadline, limits) {
        Ok(request) => {
            // The one place a request becomes attributable.
            lifecycle.log_start(&request.method, &request.path);
            let route = format!("{} {}", request.method, request.path);
            // The terminal marker is emitted *before* the panic is resumed, so a
            // panicking handler is reported as itself and then still fails the
            // connection loudly instead of being silently converted into an ordinary
            // error response. Catching without resuming would hide the fault.
            let response = match std::panic::catch_unwind(AssertUnwindSafe(|| {
                dispatch(request, service, authenticator)
            })) {
                Ok(response) => response,
                Err(payload) => {
                    lifecycle.log_panicked();
                    std::panic::resume_unwind(payload);
                }
            };
            (Some(route), response)
        }
        // The connection ended without a readable request, so no start marker can be
        // emitted for it and none is faked. A terminal marker is still emitted after
        // the write, and it is a *distinct* marker rather than a bare `request_end`:
        // a terminal line with no start line and no route is unattributable, which is
        // exactly the defect class #819 documents for the pre-existing `eprintln!`. A
        // peer that connects, says nothing, and hits the read budget is a state an
        // operator has to be able to see and tell apart from one that arrived and was
        // refused.
        //
        // The error is returned unchanged, so the `400` the peer is told and the error
        // #819's port reports are both exactly what they were before this marker
        // existed. Only the observability changed.
        Err(error) => {
            unreadable_code = Some(error.code.clone());
            (None, Err(error))
        }
    };
    // Captured before `write_response` consumes the response: this is the status the
    // peer was told, so reading it here reports what the client actually saw rather
    // than a re-derivation that could drift from `error_response`.
    let status = match &response {
        Ok(response) => response.status,
        Err(error) => error.status(),
    };
    let written = write_response(
        &mut stream,
        response,
        Instant::now() + limits.deadline,
        limits,
    );
    match (&written, &unreadable_code) {
        // A request that never parsed cannot have a start marker, and must not be
        // reported as an ordinary completion. Naming it is what keeps a peer that
        // connects and then says nothing distinguishable from one that arrived and
        // was answered.
        (_, Some(code)) => lifecycle.log_unreadable(code),
        (Ok(()), None) => lifecycle.log_end(status),
        // Only the typed code is logged. `HttpError::message` is not safe log input:
        // `io_http_error` builds it from `io::Error`, whose text can embed caller
        // bytes, so the code alone is what tells the states apart.
        (Err(error), None) => lifecycle.log_abandoned(&error.code),
    }
    written.map_err(|error| (route, error))
}
