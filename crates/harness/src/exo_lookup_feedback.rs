// SPDX-License-Identifier: MIT
//! Bounded feedback projection for the additive private duplex wire.

use serde_json::{Value, json};

use crate::exo_lookup_wire::{EXO_LOOKUP_CHUNK_BYTES, EXO_LOOKUP_FEEDBACK_BYTES};
use crate::game_information::{LookupError, LookupFeedback};

/// Compact bounded feedback survives upstream's 8,000-character tool-result wrapper.
pub(crate) fn feedback_value(
    feedback: &LookupFeedback,
    budget: usize,
) -> Result<Value, LookupError> {
    let maximum = budget.min(EXO_LOOKUP_FEEDBACK_BYTES);
    if maximum == 0 {
        return Err(LookupError::Bounds);
    }
    let value = match feedback {
        LookupFeedback::Data {
            record_ordinal,
            delivery,
        } => {
            let full = json!({"record_ordinal":record_ordinal,"data":delivery.data});
            if serde_json::to_vec(&full)
                .map_err(|_| LookupError::Invalid)?
                .len()
                <= maximum
            {
                full
            } else {
                json!({"record_ordinal":record_ordinal,"data":{"authority":"untrusted_game_information_data",
                "delivery":"retained","byte_length":delivery.record.source_bytes,
                "source_sha256":delivery.record.source_sha256}})
            }
        }
        LookupFeedback::Bootstrap {
            record_ordinal,
            response,
        } => {
            let value = json!({"record_ordinal":record_ordinal,"bootstrap":response});
            if serde_json::to_vec(&value)
                .map_err(|_| LookupError::Invalid)?
                .len()
                > maximum
            {
                return Err(LookupError::Bounds);
            }
            value
        }
        // The store's answer is already the bounded projection: it carries the page bound, the
        // declared gaps and the continuation. One answer must still fit one feedback envelope, so
        // an answer that cannot is refused rather than truncated into a shorter history. The
        // envelope bound is the one check every arm passes on its way out, so the refusal is stated
        // once rather than restated here where it could drift from the exit it depends on.
        LookupFeedback::History {
            operation_id,
            answer,
        } => json!({"operation_id":operation_id,"history":answer}),
        LookupFeedback::Bytes {
            record_ordinal,
            offset,
            total_bytes,
            bytes,
        } => {
            if *record_ordinal >= 256
                || *total_bytes > 65_536
                || offset > total_bytes
                || bytes.len() > total_bytes - offset
            {
                return Err(LookupError::Bounds);
            }
            let mut count = bytes.len().min(EXO_LOOKUP_CHUNK_BYTES);
            loop {
                let next = offset.checked_add(count).ok_or(LookupError::Bounds)?;
                let value = json!({"record_ordinal":record_ordinal,"offset":offset,"next_offset":next,
                    "total_bytes":total_bytes,"encoding":"hex","bytes":crate::hex_bytes(&bytes[..count]),
                    "authority":"untrusted_game_information_data"});
                let size = serde_json::to_vec(&value)
                    .map_err(|_| LookupError::Invalid)?
                    .len();
                if size <= maximum {
                    if count == 0 && !bytes.is_empty() {
                        return Err(LookupError::Bounds);
                    }
                    break value;
                }
                count = count
                    .checked_sub((size - maximum).div_ceil(2))
                    .ok_or(LookupError::Bounds)?;
            }
        }
        LookupFeedback::Error(error) => json!({"error":error}),
        LookupFeedback::Start => return Err(LookupError::Invalid),
    };
    if serde_json::to_vec(&value)
        .map_err(|_| LookupError::Invalid)?
        .len()
        > maximum
    {
        return Err(LookupError::Bounds);
    }
    Ok(value)
}
