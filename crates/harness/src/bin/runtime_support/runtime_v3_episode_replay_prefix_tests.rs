// SPDX-License-Identifier: MIT

use super::*;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn prefix_requires_explicit_mode_and_stops_only_at_matching_settled_checkpoint() {
    let mut values = rows();
    values.truncate(3);
    values[2]["observation"] = observation("setup", 3, "next", "ironclad");
    let encode = |values: &[Value]| {
        values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let bytes = encode(&values);
    assert!(ReplayTrace::parse(bytes.as_bytes()).is_err());
    let trace = ReplayTrace::parse_mode(bytes.as_bytes(), true).expect("settled prefix");
    let mut source = ReplaySource::new(trace);
    let fresh = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(matches!(source.decide(&fresh), Ok(Decision::Action { .. })));
    source.action_completed(true);
    assert!(
        matches!(source.decide(&fresh), Ok(Decision::Recovery { kind, .. }) if kind == "stop_episode")
    );
    assert!(source.prefix_verified);
    values[2]["observation"]["legal_actions"] = json!([]);
    assert!(ReplayTrace::parse_mode(encode(&values).as_bytes(), true).is_err());
    values.truncate(2);
    assert!(ReplayTrace::parse_mode(encode(&values).as_bytes(), true).is_err());
}

#[test]
fn verified_prefix_hands_the_same_runner_to_live_decisions_without_stop_recovery() {
    struct LiveContinuation {
        decisions: Rc<Cell<usize>>,
        completions: Rc<Cell<usize>>,
    }

    impl DecisionSource for LiveContinuation {
        fn action_completed(&mut self, settled: bool) {
            assert!(settled);
            self.completions.set(self.completions.get() + 1);
        }

        fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
            self.decisions.set(self.decisions.get() + 1);
            Ok(Decision::Wait {
                rationale: String::from("continue at verified boundary"),
            })
        }
    }

    let mut values = rows();
    values.truncate(3);
    values[2]["observation"] = observation("setup", 3, "next", "ironclad");
    values.push(json!({"event":"episode_failed","error_code":"map_snapshot_invalid"}));
    let bytes = values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let trace = ReplayTrace::parse_mode(bytes.as_bytes(), true).expect("settled replay prefix");
    let continuation_decisions = Rc::new(Cell::new(0));
    let continuation_completions = Rc::new(Cell::new(0));
    let mut continuation = LiveContinuation {
        decisions: Rc::clone(&continuation_decisions),
        completions: Rc::clone(&continuation_completions),
    };
    let boundary_published = Rc::new(Cell::new(0));
    let published = Rc::clone(&boundary_published);
    let mut publish_boundary = || {
        published.set(published.get() + 1);
        Ok(())
    };
    let mut source = ReplaySource::with_continuation(
        trace,
        Some(&mut continuation),
        Some(&mut publish_boundary),
    );
    let first_observation = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(matches!(
        source.decide(&first_observation),
        Ok(Decision::Action { .. })
    ));
    source.action_completed(true);

    let boundary = input(
        observation("setup", 21, "next-live", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(matches!(
        source.decide(&boundary),
        Ok(Decision::Wait { .. })
    ));
    assert!(source.prefix_verified);
    assert_eq!(boundary_published.get(), 1);
    assert_eq!(continuation_decisions.get(), 1);
    assert!(!matches!(
        source.decide(&boundary),
        Ok(Decision::Recovery { kind, .. }) if kind == "stop_episode"
    ));
    assert_eq!(continuation_decisions.get(), 2);
    source.action_completed(true);
    assert_eq!(continuation_completions.get(), 1);
}
