// SPDX-License-Identifier: MIT

//! Gateway-diagnostic evidence for the `served_*` compositions. Refs sts2-harness#548.
//!
//! Split out of `runtime_v4_executable_composition_process.rs` so that module stays inside
//! the repository preferred file-size budget.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// Persist the gateway's own captured streams for one served scenario, then hand the
/// failure back with those streams attached.
///
/// The `served_*` compositions never call [`write_evidence`], so before this helper the
/// gateway's stderr — the one stream that names a refused request header — was captured
/// by [`stop`], handed to the caller, and then dropped on every served path
/// (sts2-harness#548). Two things follow from that. A reader of a served failure saw a
/// `stderr=` label carrying the *workflow service's* bytes and reasonably concluded the
/// gateway's own refusal had been reported; it never had been. And the lane's
/// *"Show owned-process diagnostics"* step had nothing to print for a served step, because
/// no served step ever named an evidence directory.
///
/// This writes the gateway's streams under `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR` when
/// that variable is set, and unconditionally attaches them to the error text. It is
/// deliberately a no-op on the write path when the variable is unset so an ordinary run
/// keeps its current behaviour, and it is a separate function from [`write_evidence`]
/// because the served scenarios have no `ScenarioResult` pair and no downstream ledger to
/// summarise: they assert against the synthetic mod ledger inline instead.
pub(crate) fn gateway_failure_evidence(
    label: &str,
    gateway: &Output,
) -> Box<dyn std::error::Error> {
    if let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") {
        let root = PathBuf::from(root);
        if let Err(error) = write_gateway_streams(&root, label, gateway) {
            // Reported, not returned: the report must still carry the gateway's own streams,
            // because a persist failure is a degraded *extra* copy and dropping the in-band
            // copy too would leave a served failure explaining nothing at all.
            return format!(
                "{label}: gateway diagnostics could not be persisted under {}: {error}; \
                 gateway_stdout={}; gateway_stderr={}",
                root.display(),
                String::from_utf8_lossy(&gateway.stdout),
                String::from_utf8_lossy(&gateway.stderr),
            )
            .into();
        }
    }
    format!(
        "{label}: gateway_stdout={}; gateway_stderr={}",
        String::from_utf8_lossy(&gateway.stdout),
        String::from_utf8_lossy(&gateway.stderr),
    )
    .into()
}

/// Write one served scenario's gateway streams, so the lane's failure-only dump step has
/// bytes to print. Each served step names its own subdirectory, so `label` also keeps two
/// scenarios that share a step — the peer-acceptance step runs four negative cases and the
/// graph lane twice — from overwriting each other.
///
/// `label` arrives as the caller's full failure context, so it carries slashes, newlines and
/// other bytes that are not legal in a file name. The stream pair is therefore written under
/// a sanitised form of `label` that keeps it distinct per scenario but safe as a single path
/// component; the unsanitised text still goes into the error message itself.
fn write_gateway_streams(
    root: &Path,
    label: &str,
    gateway: &Output,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    let stem = sanitize_label(label);
    fs::write(root.join(format!("gateway-{stem}.stdout")), &gateway.stdout)?;
    fs::write(root.join(format!("gateway-{stem}.stderr")), &gateway.stderr)?;
    Ok(())
}

/// Reduce a failure context to a single safe path component.
///
/// Only `[A-Za-z0-9._-]` survive; runs of anything else become a single `_`, and the result is
/// capped so a very long context cannot produce an over-long path component. The label's
/// leading words are the scenario's own stable name (e.g. `wrong-instance`, `graph-changed`),
/// so the cap still leaves the cases that share a lane step distinguishable, which is the
/// collision this guards against.
fn sanitize_label(label: &str) -> String {
    let mut stem = String::with_capacity(label.len().min(120));
    let mut last_was_separator = false;
    for value in label.chars() {
        let keep = value.is_ascii_alphanumeric() || value == '.' || value == '-';
        if keep {
            stem.push(value);
            last_was_separator = false;
        } else if !last_was_separator {
            stem.push('_');
            last_was_separator = true;
        }
        if stem.len() >= 120 {
            break;
        }
    }
    let trimmed = stem.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "gateway".to_string()
    } else {
        trimmed
    }
}
