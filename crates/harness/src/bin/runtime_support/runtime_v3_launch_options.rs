// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::workflow_binding::{WorkflowBinding, read_replay_source};

/// Startup-only switches for the runtime-v3 gameplay workflow.
///
/// The process environment and command line are sampled once by the executable adapter. The
/// extracted workflow receives this value instead of consulting process-global selection state.
#[derive(Clone, Debug)]
pub(super) struct RuntimeV3LaunchOptions {
    pub(super) resume: bool,
    pub(super) combat_demo: bool,
    pub(super) replay_path: Option<PathBuf>,
    pub(super) replay_bytes: Option<Vec<u8>>,
    pub(super) replay_prefix: Result<bool, String>,
    pub(super) cancellation: sts2_harness::ExecutionCancellation,
}

impl RuntimeV3LaunchOptions {
    pub(super) fn from_environment() -> Result<Self, String> {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        let worker_mode = capture("STS2_WORKER_MODE").as_str() == Some("true");
        let combat_demo_env = if worker_mode {
            CapturedValue::Missing
        } else {
            capture("STS2_COMBAT_DEMO")
        };
        let replay_path_env = if worker_mode {
            CapturedValue::Missing
        } else {
            capture_path("STS2_REPLAY_TRAJECTORY")
        };
        let replay_prefix_env = if worker_mode {
            CapturedValue::Missing
        } else {
            capture("STS2_REPLAY_PREFIX")
        };
        Self::from_captured(
            arguments,
            capture("STS2_RESUME"),
            combat_demo_env,
            replay_path_env,
            replay_prefix_env,
        )
        .with_replay_source()
    }

    #[cfg(test)]
    fn from_values<I, S>(
        arguments: I,
        resume_env: Option<&str>,
        combat_demo_env: Option<&str>,
        replay_path_env: Option<&str>,
        replay_prefix_env: Option<&str>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_captured(
            arguments,
            captured(resume_env),
            captured(combat_demo_env),
            captured(replay_path_env),
            captured(replay_prefix_env),
        )
    }

    fn from_captured<I, S>(
        arguments: I,
        resume_env: CapturedValue,
        combat_demo_env: CapturedValue,
        replay_path_env: CapturedValue,
        replay_prefix_env: CapturedValue,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let resume = arguments
            .into_iter()
            .any(|argument| argument.as_ref() == "--resume")
            || resume_env.as_str() == Some("true");
        let combat_demo = combat_demo_env.as_str() == Some("true");
        let replay_path = match replay_path_env {
            CapturedValue::Text(value) if !value.is_empty() => Some(PathBuf::from(value)),
            CapturedValue::Path(path) if !path.as_os_str().is_empty() => Some(path),
            CapturedValue::Missing
            | CapturedValue::InvalidUnicode
            | CapturedValue::Text(_)
            | CapturedValue::Path(_) => None,
        };
        let replay_selected = !combat_demo && replay_path.is_some();
        let replay_prefix = match replay_prefix_env {
            CapturedValue::Text(value) if value == "true" => Ok(true),
            CapturedValue::Missing => Ok(false),
            CapturedValue::Text(value) if value == "false" => Ok(false),
            CapturedValue::InvalidUnicode | CapturedValue::Text(_) if replay_selected => {
                Err(String::from("STS2_REPLAY_PREFIX must be true or false"))
            }
            CapturedValue::InvalidUnicode | CapturedValue::Path(_) | CapturedValue::Text(_) => {
                Ok(false)
            }
        };
        Self {
            resume,
            combat_demo,
            replay_path,
            replay_bytes: None,
            replay_prefix,
            cancellation: sts2_harness::ExecutionCancellation::default(),
        }
    }

    fn with_replay_source(mut self) -> Result<Self, String> {
        self.replay_bytes = self
            .replay_path
            .as_deref()
            .map(read_replay_source)
            .transpose()?;
        Ok(self)
    }

    pub(super) fn workflow_binding(&self) -> Result<WorkflowBinding, String> {
        if self.replay_path.is_some() && self.replay_bytes.is_none() {
            return Err(String::from(
                "runtime-v3 replay selection was not loaded before durable admission",
            ));
        }
        WorkflowBinding::for_launch(
            self.combat_demo,
            self.episode_replay_prefix()?,
            self.replay_bytes.as_deref(),
        )
    }

    pub(super) fn episode_replay_prefix(&self) -> Result<bool, String> {
        self.replay_prefix.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CapturedValue {
    Missing,
    Text(String),
    Path(PathBuf),
    InvalidUnicode,
}

impl CapturedValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Missing | Self::Path(_) | Self::InvalidUnicode => None,
        }
    }
}

fn capture(name: &str) -> CapturedValue {
    match std::env::var(name) {
        Ok(value) => CapturedValue::Text(value),
        Err(std::env::VarError::NotPresent) => CapturedValue::Missing,
        Err(std::env::VarError::NotUnicode(_)) => CapturedValue::InvalidUnicode,
    }
}

fn capture_path(name: &str) -> CapturedValue {
    match std::env::var_os(name) {
        Some(value) if value.is_empty() => CapturedValue::Text(String::new()),
        Some(value) => CapturedValue::Path(PathBuf::from(value)),
        None => CapturedValue::Missing,
    }
}

#[cfg(test)]
fn captured(value: Option<&str>) -> CapturedValue {
    value.map_or(CapturedValue::Missing, |value| {
        CapturedValue::Text(value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::{CapturedValue, RuntimeV3LaunchOptions};

    #[test]
    fn command_line_resume_takes_precedence_over_false_environment_value() -> Result<(), String> {
        let options = RuntimeV3LaunchOptions::from_values(
            ["--resume"],
            Some("false"),
            Some("false"),
            Some("trajectory.jsonl"),
            Some("true"),
        );
        assert!(options.resume);
        assert!(!options.combat_demo);
        assert_eq!(
            options
                .replay_path
                .as_deref()
                .and_then(|path| path.to_str()),
            Some("trajectory.jsonl")
        );
        assert!(options.episode_replay_prefix()?);
        Ok(())
    }

    #[test]
    fn combat_demo_and_replay_are_captured_without_runtime_environment_reads() -> Result<(), String>
    {
        let options = RuntimeV3LaunchOptions::from_values(
            std::iter::empty::<&str>(),
            None,
            Some("true"),
            Some("combat.jsonl"),
            None,
        );
        assert!(options.combat_demo);
        assert_eq!(
            options
                .replay_path
                .as_deref()
                .and_then(|path| path.to_str()),
            Some("combat.jsonl")
        );
        assert!(!options.episode_replay_prefix()?);
        Ok(())
    }

    #[test]
    fn replay_prefix_rejects_values_other_than_exact_booleans() {
        for value in ["", "1", "yes", "TRUE", " true"] {
            assert!(
                RuntimeV3LaunchOptions::from_values(
                    std::iter::empty::<&str>(),
                    None,
                    None,
                    Some("trajectory.jsonl"),
                    Some(value),
                )
                .episode_replay_prefix()
                .is_err()
            );
        }
    }

    #[test]
    fn irrelevant_invalid_prefix_is_ignored_like_the_original_branch_selection()
    -> Result<(), String> {
        let options = RuntimeV3LaunchOptions::from_captured(
            std::iter::empty::<&str>(),
            CapturedValue::Missing,
            CapturedValue::Text(String::from("true")),
            CapturedValue::Text(String::from("trajectory.jsonl")),
            CapturedValue::InvalidUnicode,
        );
        assert!(options.combat_demo);
        assert!(!options.episode_replay_prefix()?);
        Ok(())
    }

    #[test]
    fn non_unicode_prefix_is_rejected_when_episode_replay_is_selected() {
        let options = RuntimeV3LaunchOptions::from_captured(
            std::iter::empty::<&str>(),
            CapturedValue::Missing,
            CapturedValue::Missing,
            CapturedValue::Text(String::from("trajectory.jsonl")),
            CapturedValue::InvalidUnicode,
        );
        assert!(options.episode_replay_prefix().is_err());
    }
}
