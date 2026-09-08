// SPDX-License-Identifier: MIT

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde_json::Value;
use sts2_harness::OperationState;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_REQUESTS: usize = 16;
const MAX_CHILD_STATUS_BYTES: usize = 64;
const MAX_DIAGNOSTIC_OUTPUT_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticStage {
    ReconcilePendingOperations,
}

impl DiagnosticStage {
    const fn as_str(self) -> &'static str {
        "reconcile_pending_operations"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryCaseTag {
    RetainedTerminal {
        lookup: RecoveryStatus,
        reconcile: RecoveryStatus,
    },
    Unresolved {
        lookup: RecoveryStatus,
        reconcile: RecoveryStatus,
    },
    WitnessDefect(WitnessDefect),
}

impl RecoveryCaseTag {
    pub(super) fn retained_terminal(lookup: &str, reconcile: &str) -> Self {
        Self::RetainedTerminal {
            lookup: RecoveryStatus::from_label(lookup),
            reconcile: RecoveryStatus::from_label(reconcile),
        }
    }

    pub(super) fn unresolved(lookup: &str, reconcile: &str) -> Self {
        Self::Unresolved {
            lookup: RecoveryStatus::from_label(lookup),
            reconcile: RecoveryStatus::from_label(reconcile),
        }
    }

    pub(super) fn witness_defect(defect: &str) -> Self {
        Self::WitnessDefect(WitnessDefect::from_label(defect))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryStatus {
    IntentRecorded,
    MayHaveBeenDispatched,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Reconciled,
    NotFound,
    Invalid,
    Unavailable,
}

impl RecoveryStatus {
    fn from_label(value: &str) -> Self {
        match value {
            "INTENT_RECORDED" => Self::IntentRecorded,
            "MAY_HAVE_BEEN_DISPATCHED" => Self::MayHaveBeenDispatched,
            "ACCEPTED" => Self::Accepted,
            "SETTLED" => Self::Settled,
            "REJECTED" => Self::Rejected,
            "UNKNOWN" => Self::Unknown,
            "RECONCILED" => Self::Reconciled,
            "NOT_FOUND" => Self::NotFound,
            _ => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WitnessDefect {
    Missing,
    OtherOperation,
    OtherContext,
    OtherFence,
    Generation,
    DuplicateWitness,
    Other,
}

impl WitnessDefect {
    fn from_label(value: &str) -> Self {
        match value {
            "missing" => Self::Missing,
            "other_operation" => Self::OtherOperation,
            "other_context" => Self::OtherContext,
            "other_fence" => Self::OtherFence,
            "generation" => Self::Generation,
            "duplicate_witness" => Self::DuplicateWitness,
            _ => Self::Other,
        }
    }
}

fn state_tag(state: Result<OperationState, String>) -> RecoveryStatus {
    match state {
        Ok(OperationState::IntentRecorded) => RecoveryStatus::IntentRecorded,
        Ok(OperationState::MayHaveBeenDispatched) => RecoveryStatus::MayHaveBeenDispatched,
        Ok(OperationState::Accepted) => RecoveryStatus::Accepted,
        Ok(OperationState::Settled) => RecoveryStatus::Settled,
        Ok(OperationState::Rejected) => RecoveryStatus::Rejected,
        Ok(OperationState::Unknown) => RecoveryStatus::Unknown,
        Ok(OperationState::Reconciled) => RecoveryStatus::Reconciled,
        Err(_) => RecoveryStatus::Unavailable,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestKind {
    Initialize,
    ToolsList,
    RecoveryLookup,
    RecoveryReconcile,
    RecoveryOther,
    GameplayObserve,
    GameplayDispatch,
    GameplayOther,
    OtherMethod,
    OtherTool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestSummaryStatus {
    Missing,
    Unavailable,
    Oversized,
    Malformed,
    Truncated,
    Parsed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestSummary {
    status: RequestSummaryStatus,
    entries: [RequestKind; MAX_REQUESTS],
    len: usize,
}

impl RequestSummary {
    fn status(status: RequestSummaryStatus) -> Self {
        Self {
            status,
            entries: [RequestKind::OtherMethod; MAX_REQUESTS],
            len: 0,
        }
    }

    fn parsed(entries: [RequestKind; MAX_REQUESTS], len: usize) -> Self {
        Self {
            status: RequestSummaryStatus::Parsed,
            entries,
            len,
        }
    }

    fn label(self) -> String {
        if self.status != RequestSummaryStatus::Parsed {
            return format!("{:?}", self.status);
        }
        let mut label = String::from("parsed[");
        for (index, entry) in self.entries[..self.len].iter().enumerate() {
            if index != 0 {
                label.push(',');
            }
            let _ = write!(label, "{entry:?}");
        }
        label.push(']');
        label
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildStatus {
    Success,
    Failure,
    Missing,
    Malformed,
    Oversized,
    Unavailable,
}

pub(super) fn emit_failure(
    requests_path: &Path,
    child_status_path: &Path,
    case: RecoveryCaseTag,
    durable_state: Result<OperationState, String>,
) {
    let report = render_failure(
        case,
        state_tag(durable_state),
        summarize_requests_path(requests_path),
        summarize_child_status_path(child_status_path),
    );
    eprintln!("{report}");
}

fn render_failure(
    case: RecoveryCaseTag,
    durable_state: RecoveryStatus,
    requests: RequestSummary,
    child_status: ChildStatus,
) -> String {
    let mut report = String::with_capacity(256);
    report.push_str("runtime-v3 recovery diagnostic ");
    report.push_str("version=1 ");
    report.push_str("stage=");
    report.push_str(DiagnosticStage::ReconcilePendingOperations.as_str());
    // The test boundary exposes only a string, so this must not infer an inner error variant.
    report.push_str(" failure=recovery_failed ");
    report.push_str("case=");
    match case {
        RecoveryCaseTag::RetainedTerminal { lookup, reconcile } => {
            let _ = write!(
                report,
                "retained_terminal,lookup={lookup:?},reconcile={reconcile:?}"
            );
        }
        RecoveryCaseTag::Unresolved { lookup, reconcile } => {
            let _ = write!(
                report,
                "unresolved,lookup={lookup:?},reconcile={reconcile:?}"
            );
        }
        RecoveryCaseTag::WitnessDefect(defect) => {
            let _ = write!(report, "witness_defect,{defect:?}");
        }
    }
    let _ = write!(
        report,
        " durable_state={durable_state:?} requests={} child_status={child_status:?}",
        requests.label(),
    );
    bound_output(report)
}

fn bound_output(mut report: String) -> String {
    if report.len() <= MAX_DIAGNOSTIC_OUTPUT_BYTES {
        return report;
    }
    report.truncate(MAX_DIAGNOSTIC_OUTPUT_BYTES - 10);
    report.push_str(" truncated");
    report
}

fn summarize_requests_path(path: &Path) -> RequestSummary {
    match read_capped(path, MAX_REQUEST_BYTES) {
        CappedBytes::Missing => RequestSummary::status(RequestSummaryStatus::Missing),
        CappedBytes::Unavailable => RequestSummary::status(RequestSummaryStatus::Unavailable),
        CappedBytes::Oversized => RequestSummary::status(RequestSummaryStatus::Oversized),
        CappedBytes::Ready(bytes) => summarize_requests(&bytes),
    }
}

fn summarize_requests(bytes: &[u8]) -> RequestSummary {
    let mut entries = [RequestKind::OtherMethod; MAX_REQUESTS];
    let mut len = 0;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if len == MAX_REQUESTS {
            return RequestSummary::status(RequestSummaryStatus::Truncated);
        }
        let value = match serde_json::from_slice::<Value>(line) {
            Ok(value) => value,
            Err(_) => return RequestSummary::status(RequestSummaryStatus::Malformed),
        };
        entries[len] = classify_request(&value);
        len += 1;
    }
    RequestSummary::parsed(entries, len)
}

fn classify_request(value: &Value) -> RequestKind {
    match value.get("method").and_then(Value::as_str) {
        Some("initialize") => RequestKind::Initialize,
        Some("tools/list") => RequestKind::ToolsList,
        Some("tools/call") => classify_tool(value),
        Some(_) => RequestKind::OtherMethod,
        None => RequestKind::OtherMethod,
    }
}

fn classify_tool(value: &Value) -> RequestKind {
    match value
        .get("params")
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
    {
        Some("watchdog.operation_lookup") => RequestKind::RecoveryLookup,
        Some("watchdog.operation_reconcile") => RequestKind::RecoveryReconcile,
        Some(name) if name.starts_with("watchdog.") => RequestKind::RecoveryOther,
        Some("sts2.observe") => RequestKind::GameplayObserve,
        Some("sts2.dispatch_action") => RequestKind::GameplayDispatch,
        Some(name) if name.starts_with("sts2.") => RequestKind::GameplayOther,
        Some(_) | None => RequestKind::OtherTool,
    }
}

fn summarize_child_status_path(path: &Path) -> ChildStatus {
    match read_capped(path, MAX_CHILD_STATUS_BYTES) {
        CappedBytes::Missing => ChildStatus::Missing,
        CappedBytes::Unavailable => ChildStatus::Unavailable,
        CappedBytes::Oversized => ChildStatus::Oversized,
        CappedBytes::Ready(bytes) => parse_child_status(&bytes),
    }
}

fn parse_child_status(bytes: &[u8]) -> ChildStatus {
    let value = match bytes.strip_suffix(b"\n") {
        Some(value) => value,
        None => bytes,
    };
    let Some(code) = value.strip_prefix(b"exit=") else {
        return ChildStatus::Malformed;
    };
    if code == b"0" {
        return ChildStatus::Success;
    }
    if code.is_empty() || code.len() > 3 || !code.iter().all(u8::is_ascii_digit) {
        return ChildStatus::Malformed;
    }
    match code.iter().try_fold(0_u16, |value, digit| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u16::from(digit - b'0')))
    }) {
        Some(1..=255) => ChildStatus::Failure,
        _ => ChildStatus::Malformed,
    }
}

enum CappedBytes {
    Missing,
    Unavailable,
    Oversized,
    Ready(Vec<u8>),
}

fn read_capped(path: &Path, limit: usize) -> CappedBytes {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return CappedBytes::Missing,
        Err(_) => return CappedBytes::Unavailable,
    };
    let mut bytes = Vec::with_capacity(limit.saturating_add(1));
    let Ok(read_limit) = u64::try_from(limit.saturating_add(1)) else {
        return CappedBytes::Unavailable;
    };
    if file.take(read_limit).read_to_end(&mut bytes).is_err() {
        return CappedBytes::Unavailable;
    }
    if bytes.len() > limit {
        CappedBytes::Oversized
    } else {
        CappedBytes::Ready(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_request_input_is_redacted_and_bounded() {
        let bytes = br#"{"method":"tools/call","params":{"name":"secret","payload":"/private/path"}} trailing"#;
        let summary = summarize_requests(bytes);
        assert_eq!(summary.status, RequestSummaryStatus::Malformed);
        assert_eq!(summary.label(), "Malformed");
    }

    #[test]
    fn oversized_files_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        use super::super::super::reconnect_support::Fixture;

        let fixture = Fixture::new()?;
        let requests_path = fixture.0.join("requests");
        std::fs::write(&requests_path, vec![1_u8; MAX_REQUEST_BYTES])?;
        let exact = summarize_requests_path(&requests_path);
        assert_eq!(exact.status, RequestSummaryStatus::Malformed);
        std::fs::write(&requests_path, vec![1_u8; MAX_REQUEST_BYTES + 1])?;
        let summary = summarize_requests_path(&requests_path);
        assert_eq!(summary.status, RequestSummaryStatus::Oversized);

        let child_path = fixture.0.join("child-status");
        std::fs::write(&child_path, vec![2_u8; MAX_CHILD_STATUS_BYTES])?;
        assert!(matches!(
            summarize_child_status_path(&child_path),
            ChildStatus::Malformed
        ));
        std::fs::write(&child_path, vec![2_u8; MAX_CHILD_STATUS_BYTES + 1])?;
        let child_status = summarize_child_status_path(&child_path);
        assert_eq!(child_status, ChildStatus::Oversized);
        let report = render_failure(
            RecoveryCaseTag::unresolved("UNKNOWN", "NOT_FOUND"),
            RecoveryStatus::Unknown,
            summary,
            child_status,
        );
        assert!(report.len() <= MAX_DIAGNOSTIC_OUTPUT_BYTES);
        assert!(!report.contains('\u{1}') && !report.contains('\u{2}'));
        Ok(())
    }

    #[test]
    fn malformed_and_excessive_child_status_is_fixed() {
        assert!(matches!(
            parse_child_status(b"exit=wat secret"),
            ChildStatus::Malformed
        ));
        assert_eq!(parse_child_status(b"exit=1\n"), ChildStatus::Failure);
    }
}
