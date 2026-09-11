// SPDX-License-Identifier: MIT

use super::analysis::{CountStatus, LegalDestinationPathCount, TerminalPathCount};
use super::graph::{MapCompleteness, ValidatedMapGraph};
use std::collections::BTreeMap;
use std::fmt;

pub(crate) fn terminal_counts(
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
    order: &[String],
    max_digits: usize,
) -> Vec<TerminalPathCount> {
    let terminal_ids = graph
        .terminals()
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut paths = graph
        .nodes()
        .iter()
        .map(|node| (node.node_id.clone(), DecimalCount::zero(max_digits)))
        .collect::<BTreeMap<_, _>>();
    if let Some(current) = graph.current_node() {
        if let Some(value) = paths.get_mut(current) {
            *value = DecimalCount::one(max_digits);
        }
        for node in order {
            if terminal_ids.contains(node) {
                continue;
            }
            let value = paths
                .get(node)
                .cloned()
                .unwrap_or_else(|| DecimalCount::zero(max_digits));
            if let Some(next) = adjacency.get(node) {
                for child in next {
                    if let Some(target) = paths.get_mut(child) {
                        target.add_assign(&value);
                    }
                }
            }
        }
    }
    graph
        .terminals()
        .iter()
        .map(|terminal| {
            let value = paths
                .get(terminal)
                .cloned()
                .unwrap_or_else(|| DecimalCount::zero(max_digits));
            let status = if !matches!(graph.completeness(), MapCompleteness::Complete) {
                CountStatus::Incomplete
            } else if value.overflow {
                CountStatus::Overflow
            } else {
                CountStatus::Exact
            };
            TerminalPathCount {
                terminal_id: terminal.clone(),
                count_decimal: value.to_string(),
                status,
            }
        })
        .collect()
}

pub(crate) fn legal_destination_counts(
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
    order: &[String],
    max_digits: usize,
) -> Vec<LegalDestinationPathCount> {
    let terminal_ids = graph
        .terminals()
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut result = Vec::with_capacity(graph.legal_destinations().len());
    for destination in graph.legal_destinations() {
        let bound = graph.current_node().is_some_and(|current| {
            current == destination.node_id
                || adjacency
                    .get(current)
                    .is_some_and(|next| next.binary_search(&destination.node_id).is_ok())
        });
        let (count_decimal, status) = if !bound {
            ("0".to_owned(), CountStatus::Incomplete)
        } else {
            let counts = path_counts_from(
                &destination.node_id,
                adjacency,
                order,
                max_digits,
                &terminal_ids,
            );
            let mut total = DecimalCount::zero(max_digits);
            for terminal in &terminal_ids {
                if let Some(value) = counts.get(terminal) {
                    total.add_assign(value);
                }
            }
            let status = if !matches!(graph.completeness(), MapCompleteness::Complete) {
                CountStatus::Incomplete
            } else if total.overflow {
                CountStatus::Overflow
            } else {
                CountStatus::Exact
            };
            (total.to_string(), status)
        };
        result.push(LegalDestinationPathCount {
            destination_node_id: destination.node_id.clone(),
            action_id: destination.action_id.clone(),
            count_decimal,
            status,
        });
    }
    result
}

fn path_counts_from(
    source: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    order: &[String],
    max_digits: usize,
    terminals: &std::collections::BTreeSet<String>,
) -> BTreeMap<String, DecimalCount> {
    let mut counts = adjacency
        .keys()
        .map(|node| (node.clone(), DecimalCount::zero(max_digits)))
        .collect::<BTreeMap<_, _>>();
    if let Some(value) = counts.get_mut(source) {
        *value = DecimalCount::one(max_digits);
    }
    for node in order {
        if terminals.contains(node) {
            continue;
        }
        let value = counts
            .get(node)
            .cloned()
            .unwrap_or_else(|| DecimalCount::zero(max_digits));
        if let Some(next) = adjacency.get(node) {
            for child in next {
                if let Some(target) = counts.get_mut(child) {
                    target.add_assign(&value);
                }
            }
        }
    }
    counts
}

#[derive(Clone, Debug)]
struct DecimalCount {
    digits: Vec<u8>,
    limit: usize,
    overflow: bool,
}

impl DecimalCount {
    fn zero(limit: usize) -> Self {
        Self {
            digits: vec![0],
            limit,
            overflow: false,
        }
    }

    fn one(limit: usize) -> Self {
        Self {
            digits: vec![1],
            limit,
            overflow: false,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        if self.overflow || other.overflow {
            self.overflow = true;
            return;
        }
        let length = self.digits.len().max(other.digits.len());
        self.digits.resize(length, 0);
        let mut carry = 0_u8;
        for index in 0..length {
            let right = other.digits.get(index).copied().unwrap_or(0);
            let sum = self.digits[index] + right + carry;
            self.digits[index] = sum % 10;
            carry = sum / 10;
        }
        if carry > 0 {
            self.digits.push(carry);
        }
        if self.digits.len() > self.limit {
            self.overflow = true;
        }
    }
}

impl fmt::Display for DecimalCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.overflow {
            return formatter.write_str("overflow");
        }
        let value: String = self
            .digits
            .iter()
            .rev()
            .map(|digit| char::from(b'0' + *digit))
            .collect();
        formatter.write_str(&value)
    }
}
