// SPDX-License-Identifier: MIT

//! Verify actual static checkout steps, never YAML-looking text in shell bodies.

use yaml_rust2::{Yaml, YamlLoader};

#[path = "workflow_yaml_shape.rs"]
mod yaml_shape;

pub(super) fn checkout_pin_matches(source: &str, repository: &str, revision: &str) -> bool {
    if !yaml_shape::unambiguous_bounded_document(source) {
        return false;
    }
    let Ok(documents) = YamlLoader::load_from_str(source) else {
        return false;
    };
    let [document] = documents.as_slice() else {
        return false;
    };
    let Some(jobs) = document["jobs"].as_hash() else {
        return false;
    };
    jobs.values()
        .any(|job| job_has_checkout(job, repository, revision))
}

fn job_has_checkout(job: &Yaml, repository: &str, revision: &str) -> bool {
    // Deliberately support static jobs only: a conditional, dependent or empty-matrix
    // job cannot establish that this checkout belongs to the advertised CI lane.
    if job.as_hash().is_none()
        || ["if", "needs", "strategy", "uses"]
            .iter()
            .any(|field| !job[*field].is_badvalue())
        || !failure_is_required(job)
        || !job["runs-on"]
            .as_str()
            .is_some_and(|runner| !runner.is_empty() && !runner.contains("${{"))
    {
        return false;
    }
    job["steps"].as_vec().is_some_and(|steps| {
        steps
            .iter()
            .any(|step| checkout_matches(step, repository, revision))
    })
}

fn checkout_matches(step: &Yaml, repository: &str, revision: &str) -> bool {
    if step.as_hash().is_none()
        || !step["if"].is_badvalue()
        || !step["run"].is_badvalue()
        || !failure_is_required(step)
    {
        return false;
    }
    let Some(action_revision) = step["uses"]
        .as_str()
        .and_then(|action| action.strip_prefix("actions/checkout@"))
    else {
        return false;
    };
    let immutable = action_revision.len() == 40
        && action_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    immutable
        && step["with"].as_hash().is_some()
        && step["with"]["repository"].as_str() == Some(repository)
        && step["with"]["ref"].as_str() == Some(revision)
}

fn failure_is_required(node: &Yaml) -> bool {
    node["continue-on-error"].is_badvalue() || node["continue-on-error"] == Yaml::Boolean(false)
}

#[cfg(test)]
#[path = "workflow_checkout_tests.rs"]
mod tests;
