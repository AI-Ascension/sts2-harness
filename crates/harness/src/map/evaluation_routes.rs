// SPDX-License-Identifier: MIT

use super::evaluation::SYNTHETIC_MAX_ROUTE_NODES;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn terminal_route(
    node: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    terminals: &BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    memo: &mut BTreeMap<String, Option<Vec<String>>>,
) -> Option<Vec<String>> {
    if let Some(route) = memo.get(node) {
        return route.clone();
    }
    if visited.len() >= SYNTHETIC_MAX_ROUTE_NODES.saturating_sub(1) {
        return None;
    }
    if !visited.insert(node.to_owned()) {
        return None;
    }
    let route = if terminals.contains(node) {
        Some(vec![node.to_owned()])
    } else {
        let mut best = adjacency
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|child| {
                terminal_route(child, adjacency, terminals, visited, memo).map(|suffix| {
                    let mut route = Vec::with_capacity(suffix.len() + 1);
                    route.push(node.to_owned());
                    route.extend(suffix);
                    route
                })
            })
            .collect::<Vec<_>>();
        best.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
        best.into_iter().next()
    };
    visited.remove(node);
    memo.insert(node.to_owned(), route.clone());
    route
}
