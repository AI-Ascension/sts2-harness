// SPDX-License-Identifier: MIT

use super::canonical::reject_duplicate_keys;
use super::graph::{
    LegalDestination, MapCompleteness, MapEdge, MapGraphError, MapNode, MapNodeStatus,
    ValidatedMapGraph,
};

impl ValidatedMapGraph {
    /// Adapts one protocol-owned `visible-map-v1` snapshot JSON into the
    /// analysis input. The protocol decoder remains the authority for the
    /// complete envelope; this method only maps validated snapshot fields.
    pub fn from_visible_map_json(
        snapshot_digest: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Self, MapGraphError> {
        reject_duplicate_keys(bytes)
            .map_err(|_| MapGraphError::Wire("duplicate or invalid JSON"))?;
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| MapGraphError::Wire("snapshot JSON"))?;
        let object = value
            .as_object()
            .ok_or(MapGraphError::Wire("snapshot object"))?;
        let snapshot_digest = snapshot_digest.into();
        if let Some(declared) = object
            .get("snapshot_digest")
            .and_then(serde_json::Value::as_str)
            && declared != snapshot_digest
        {
            return Err(MapGraphError::Wire("snapshot_digest"));
        }
        let generation = object
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .ok_or(MapGraphError::Wire("generation"))?;
        let mut completeness = match object
            .get("completeness")
            .and_then(serde_json::Value::as_str)
            .ok_or(MapGraphError::Wire("completeness"))?
        {
            "complete" => MapCompleteness::Complete,
            "incomplete" | "unknown" => MapCompleteness::Incomplete,
            _ => return Err(MapGraphError::Wire("completeness")),
        };
        if object
            .get("availability")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value != "available")
        {
            completeness = MapCompleteness::Unavailable;
        } else if object
            .get("freshness")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value != "current")
        {
            completeness = MapCompleteness::Incomplete;
        }
        let nodes = object
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .ok_or(MapGraphError::Wire("nodes"))?
            .iter()
            .map(parse_node)
            .collect::<Result<Vec<_>, _>>()?;
        let edges = object
            .get("edges")
            .and_then(serde_json::Value::as_array)
            .ok_or(MapGraphError::Wire("edges"))?
            .iter()
            .map(parse_edge)
            .collect::<Result<Vec<_>, _>>()?;
        let current_node = if let Some(value) = object.get("current_node") {
            match value {
                serde_json::Value::Null => None,
                serde_json::Value::String(value) => Some(value.clone()),
                _ => return Err(MapGraphError::Wire("current_node")),
            }
        } else {
            let position = object
                .get("position")
                .and_then(serde_json::Value::as_object)
                .ok_or(MapGraphError::Wire("position"))?;
            match position
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .ok_or(MapGraphError::Wire("position.kind"))?
            {
                "current" => Some(
                    position
                        .get("node_id")
                        .and_then(serde_json::Value::as_str)
                        .ok_or(MapGraphError::Wire("position.node_id"))?
                        .to_owned(),
                ),
                "pre_start" | "unavailable" => None,
                _ => return Err(MapGraphError::Wire("position.kind")),
            }
        };
        let legal_destinations = object
            .get("legal_bindings")
            .or_else(|| object.get("bindings"))
            .and_then(serde_json::Value::as_array)
            .ok_or(MapGraphError::Wire("legal_bindings"))?
            .iter()
            .map(parse_binding)
            .collect::<Result<Vec<_>, _>>()?;
        let terminals = object
            .get("terminals")
            .or_else(|| object.get("terminal_node_ids"))
            .and_then(serde_json::Value::as_array)
            .ok_or(MapGraphError::Wire("terminals"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(MapGraphError::Wire("terminal_node_id"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let map_instance = object
            .get("map_instance")
            .or_else(|| object.get("map_instance_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or(MapGraphError::Wire("map_instance"))?;
        let act = object
            .get("act")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                object
                    .get("act_id")
                    .and_then(serde_json::Value::as_u64)
                    .map(|value| value.to_string())
            })
            .ok_or(MapGraphError::Wire("act"))?;
        ValidatedMapGraph::new(
            snapshot_digest,
            map_instance,
            act,
            object
                .get("source_state_id")
                .or_else(|| object.get("state_id"))
                .and_then(serde_json::Value::as_str)
                .ok_or(MapGraphError::Wire("source_state_id"))?,
            generation,
            completeness,
            nodes,
            edges,
            current_node,
            legal_destinations,
            terminals,
        )
    }
}

fn parse_node(value: &serde_json::Value) -> Result<MapNode, MapGraphError> {
    let object = value.as_object().ok_or(MapGraphError::Wire("node"))?;
    let category = object
        .get("category")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let status = object
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || {
                if object
                    .get("visited")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    MapNodeStatus::Visited
                } else {
                    MapNodeStatus::Unknown
                }
            },
            |status| match status {
                "current" => MapNodeStatus::Current,
                "visited" => MapNodeStatus::Visited,
                "available" => MapNodeStatus::Available,
                "unavailable" => MapNodeStatus::Unavailable,
                _ => MapNodeStatus::Unknown,
            },
        );
    Ok(MapNode {
        node_id: string_field(object, "id")?,
        row: integer_field(object, "row")?,
        column: integer_field(object, "column")?,
        category,
        status,
    })
}

fn parse_edge(value: &serde_json::Value) -> Result<MapEdge, MapGraphError> {
    let object = value.as_object().ok_or(MapGraphError::Wire("edge"))?;
    Ok(MapEdge {
        from: string_field(object, "from")?,
        to: string_field(object, "to")?,
    })
}

fn parse_binding(value: &serde_json::Value) -> Result<LegalDestination, MapGraphError> {
    let object = value.as_object().ok_or(MapGraphError::Wire("binding"))?;
    let (node_id, action_id) = if object.contains_key("node_id") {
        (
            string_field(object, "node_id")?,
            string_field(object, "action_id")?,
        )
    } else {
        let node_id = string_field(object, "graph_node_id")?;
        let action_id = string_field(object, "host_action_id")?;
        let action = object
            .get("action")
            .and_then(serde_json::Value::as_object)
            .ok_or(MapGraphError::Wire("binding.action"))?;
        if action.get("kind").and_then(serde_json::Value::as_str) != Some("select_map_node")
            || action
                .get("node_id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(MapGraphError::Wire("binding action payload"));
        }
        (node_id, action_id)
    };
    Ok(LegalDestination { node_id, action_id })
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<String, MapGraphError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(MapGraphError::Wire(field))
}

fn integer_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<i32, MapGraphError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(MapGraphError::Wire(field))
}
