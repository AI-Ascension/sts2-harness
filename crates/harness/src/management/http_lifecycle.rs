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
//! **A request that was served emits a start line and exactly one terminal line, and
//! the absence of the terminal line is itself the signal that it did not finish.**
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
//! - The terminal marker is emitted on every path out of this function. A marker that
//!   can be skipped by a panic or an early return would recreate exactly the silence
//!   this work exists to remove.

use std::net::TcpStream;
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
/// Returns the same result the connection worker already handles, so this is purely
/// additive to the existing failure path: #819's `eprintln!` still sees exactly the
/// errors it saw before.
pub(super) fn serve_reported(
    mut stream: TcpStream,
    service: &ManagementService,
    authenticator: &dyn Authenticator,
    limits: &HttpLimits,
    lifecycle_log: &RequestLifecycleLog,
) -> Result<(), HttpError> {
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
    let response = match read_request(&mut stream, read_deadline, limits) {
        Ok(request) => {
            // The one place a request becomes attributable.
            lifecycle.log_start(&request.method, &request.path);
            dispatch(request, service, authenticator)
        }
        Err(error) => Err(error),
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
    match &written {
        Ok(()) => lifecycle.log_end(status),
        // Only the typed code is logged. `HttpError::message` is not safe log input:
        // `io_http_error` builds it from `io::Error`, whose text can embed caller
        // bytes, so the code alone is what tells the states apart.
        Err(error) => lifecycle.log_abandoned(&error.code),
    }
    written
}
