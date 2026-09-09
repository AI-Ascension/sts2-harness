// SPDX-License-Identifier: MIT

//! Completion-aware quarantine and lossless cleanup error composition.

use super::RuntimeV3Port;

pub(super) fn combine_quarantine(error: String, quarantine: Result<(), String>) -> String {
    match quarantine {
        Ok(()) => error,
        Err(quarantine_error) => {
            format!("{error}; failed to persist interrupted-unknown quarantine: {quarantine_error}")
        }
    }
}

pub(super) fn combine_store_close(error: String, close: Result<(), String>) -> String {
    match close {
        Ok(()) => error,
        Err(close_error) => format!("{error}; execution store close failed: {close_error}"),
    }
}

pub(super) fn quarantine_unless_completed(
    port: &RuntimeV3Port,
    reason: &str,
) -> Result<(), String> {
    match port.durable_handle() {
        Some(durable) if durable.is_completed()? => Ok(()),
        Some(_) | None => port.mark_interrupted_unknown(reason),
    }
}

pub(super) fn finish_cleanup(
    result: Result<(), String>,
    cleanup: Result<(), String>,
    label: &str,
) -> Result<(), String> {
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(format!("{label}: {error}")),
        (Err(original), Err(error)) => Err(format!("{original}; {label}: {error}")),
    }
}
