// SPDX-License-Identifier: MIT

use super::analysis::{
    ApproximationStatus, CandidateRoute, CountStatus, LegalDestinationPathCount, RoutePolicy,
    RouteScore, TerminalPathCount,
};
use super::graph::{MapCompleteness, ValidatedMapGraph};
use std::collections::BTreeMap;

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

pub(crate) fn candidate_routes(
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
    distance_to_terminal: &BTreeMap<String, u32>,
    max_candidates: usize,
    max_route_nodes: usize,
    policy: &RoutePolicy,
) -> (Vec<CandidateRoute>, bool) {
    let action_by_node = graph
        .legal_destinations()
        .iter()
        .map(|destination| (destination.node_id.as_str(), destination.action_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let category_by_node = graph
        .nodes()
        .iter()
        .map(|node| (node.node_id.as_str(), node.category.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut routes = Vec::new();
    let enumeration_limit = max_candidates.saturating_add(1);
    for destination in graph.legal_destinations() {
        let Some(current) = graph.current_node() else {
            continue;
        };
        if destination.node_id != current
            && !adjacency
                .get(current)
                .is_some_and(|next| next.binary_search(&destination.node_id).is_ok())
        {
            continue;
        }
        if !distance_to_terminal.contains_key(&destination.node_id) {
            continue;
        }
        let mut path = graph
            .current_node()
            .map_or_else(Vec::new, |current| vec![current.to_owned()]);
        if destination.node_id != current {
            path.push(destination.node_id.clone());
        }
        enumerate_routes(
            &destination.node_id,
            adjacency,
            graph.terminals(),
            max_route_nodes,
            enumeration_limit,
            &mut path,
            &mut routes,
            action_by_node
                .get(destination.node_id.as_str())
                .copied()
                .unwrap_or(""),
            distance_to_terminal,
            policy,
            &category_by_node,
        );
        if routes.len() >= enumeration_limit {
            break;
        }
    }
    let bounded = routes.len() > max_candidates;
    routes.sort_by(|left, right| {
        (
            left.score.distance_to_terminal,
            std::cmp::Reverse(left.score.category_score),
            std::cmp::Reverse(left.score.retained_branching),
            left.tie_break_key.as_str(),
        )
            .cmp(&(
                right.score.distance_to_terminal,
                std::cmp::Reverse(right.score.category_score),
                std::cmp::Reverse(right.score.retained_branching),
                right.tie_break_key.as_str(),
            ))
    });
    routes.truncate(max_candidates);
    for route in &mut routes {
        route.selection = if bounded {
            ApproximationStatus::BoundedCandidates
        } else {
            ApproximationStatus::Exact
        };
    }
    (routes, bounded)
}

fn enumerate_routes(
    node: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    terminals: &[String],
    max_route_nodes: usize,
    route_limit: usize,
    path: &mut Vec<String>,
    routes: &mut Vec<CandidateRoute>,
    first_action_id: &str,
    distance_to_terminal: &BTreeMap<String, u32>,
    policy: &RoutePolicy,
    category_by_node: &BTreeMap<&str, &str>,
) {
    if routes.len() >= route_limit || path.len() > max_route_nodes {
        return;
    }
    if terminals.iter().any(|terminal| terminal == node) {
        let retained_branching = path
            .iter()
            .filter(|value| {
                adjacency
                    .get(value.as_str())
                    .is_some_and(|next| next.len() > 1)
            })
            .count() as u32;
        let tie_break_key = path.join("\u{1f}");
        routes.push(CandidateRoute {
            nodes: path.clone(),
            first_action_id: first_action_id.to_owned(),
            score: RouteScore {
                distance_to_terminal: path
                    .len()
                    .checked_sub(1)
                    .and_then(|length| u32::try_from(length).ok()),
                category_score: route_category_score(&path, category_by_node, policy),
                rest_count: route_category_count(&path, category_by_node, Category::Rest),
                shop_count: route_category_count(&path, category_by_node, Category::Shop),
                elite_count: route_category_count(&path, category_by_node, Category::Elite),
                elite_exposure_before_rest: elite_exposure_before_rest(&path, category_by_node),
                retained_branching,
                route_length: path.len() as u32,
            },
            tie_break_key,
            selection: ApproximationStatus::Exact,
            assumptions: vec!["candidate is a structural route, not a reward estimate".to_owned()],
        });
        return;
    }
    let Some(next) = adjacency.get(node) else {
        return;
    };
    for child in next {
        path.push(child.clone());
        enumerate_routes(
            child,
            adjacency,
            terminals,
            max_route_nodes,
            route_limit,
            path,
            routes,
            first_action_id,
            distance_to_terminal,
            policy,
            category_by_node,
        );
        path.pop();
        if routes.len() >= route_limit {
            return;
        }
    }
}

#[derive(Clone, Copy)]
enum Category {
    Rest,
    Shop,
    Elite,
}

fn route_category_score(
    path: &[String],
    category_by_node: &BTreeMap<&str, &str>,
    policy: &RoutePolicy,
) -> i32 {
    let rest_count = route_category_count(path, category_by_node, Category::Rest);
    let shop_count = route_category_count(path, category_by_node, Category::Shop);
    let elite_count = route_category_count(path, category_by_node, Category::Elite);
    let elite_exposure_before_rest = elite_exposure_before_rest(path, category_by_node);
    let category_score = path
        .iter()
        .filter_map(|node_id| category_by_node.get(node_id.as_str()))
        .filter_map(|category| policy.category_weights.get(*category).copied())
        .sum::<i32>();
    category_score
        .saturating_add(
            i32::try_from(rest_count)
                .unwrap_or(i32::MAX)
                .saturating_mul(policy.rest_weight),
        )
        .saturating_add(
            i32::try_from(shop_count)
                .unwrap_or(i32::MAX)
                .saturating_mul(policy.shop_weight),
        )
        .saturating_add(
            i32::try_from(elite_count)
                .unwrap_or(i32::MAX)
                .saturating_mul(policy.elite_weight),
        )
        .saturating_add(
            i32::try_from(elite_exposure_before_rest)
                .unwrap_or(i32::MAX)
                .saturating_mul(policy.elite_exposure_before_rest_weight),
        )
}

fn route_category_count(
    path: &[String],
    category_by_node: &BTreeMap<&str, &str>,
    category: Category,
) -> u32 {
    path.iter()
        .filter_map(|node_id| category_by_node.get(node_id.as_str()).copied())
        .filter(|value| match category {
            Category::Rest => is_rest_category(value),
            Category::Shop => is_shop_category(value),
            Category::Elite => is_elite_category(value),
        })
        .count() as u32
}

fn elite_exposure_before_rest(path: &[String], category_by_node: &BTreeMap<&str, &str>) -> u32 {
    path.iter()
        .map(|node_id| {
            category_by_node
                .get(node_id.as_str())
                .copied()
                .unwrap_or("")
        })
        .take_while(|category| !is_rest_category(category))
        .filter(|category| is_elite_category(category))
        .count() as u32
}

fn is_rest_category(category: &str) -> bool {
    matches!(category, "rest" | "rest_site" | "rest-site" | "campfire")
}

fn is_shop_category(category: &str) -> bool {
    matches!(
        category,
        "shop" | "merchant" | "merchant_preview" | "merchant-preview"
    )
}

fn is_elite_category(category: &str) -> bool {
    category == "elite"
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

    fn to_string(&self) -> String {
        if self.overflow {
            return "overflow".to_owned();
        }
        self.digits
            .iter()
            .rev()
            .map(|digit| char::from(b'0' + *digit))
            .collect()
    }
}
