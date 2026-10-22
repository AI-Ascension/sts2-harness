// SPDX-License-Identifier: MIT
//! Closed selectors for the host-owned complete catalog and its separately retained bytes.
use crate::lookup_wire::{Payload, valid_id};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestArguments {
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    record_ordinal: usize,
    offset: usize,
}

pub fn payload(arguments: Value, read: bool) -> Result<Payload, &'static str> {
    if read {
        let args: ReadArguments =
            serde_json::from_value(arguments).map_err(|_| "exo_lookup_arguments")?;
        if args.record_ordinal >= 32 || args.offset > 131_072 {
            return Err("exo_lookup_arguments");
        }
        Ok(Payload::ReadManifest {
            record_ordinal: args.record_ordinal,
            offset: args.offset,
        })
    } else {
        let args: ManifestArguments =
            serde_json::from_value(arguments).map_err(|_| "exo_lookup_arguments")?;
        if !valid_id(&args.operation_id) || args.operation_id.len() > 64 {
            return Err("exo_lookup_arguments");
        }
        Ok(Payload::ContentManifest {
            operation_id: args.operation_id,
        })
    }
}
