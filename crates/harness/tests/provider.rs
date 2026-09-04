// SPDX-License-Identifier: MIT

use sts2_harness::{
    Correlation, IdempotencyKey, ModelExecutionId, ModelOutput, ModelRequest, ModelResponse,
    Prompt, RetryPolicy,
};

fn correlation(execution_id: ModelExecutionId) -> Correlation {
    Correlation::for_episode(
        sts2_harness::RunId::new(1).expect("run ID is nonzero"),
        sts2_harness::EpisodeId::new(2).expect("episode ID is nonzero"),
        sts2_harness::TrajectoryId::new(3).expect("trajectory ID is nonzero"),
        sts2_harness::InstanceId::new(4).expect("instance ID is nonzero"),
        sts2_harness::TraceId::new(5).expect("trace ID is nonzero"),
    )
    .with_model_execution(execution_id)
}

#[test]
fn provider_request_is_bounded_and_identity_carrying() {
    let execution_id = ModelExecutionId::new(7).expect("execution ID is nonzero");
    let prompt = Prompt::new("choose one current action").expect("prompt is valid");
    let key = IdempotencyKey::new("model-request-7").expect("idempotency key is valid");
    let request = ModelRequest::new(execution_id, correlation(execution_id), prompt, key);

    assert_eq!(request.execution_id(), execution_id);
    assert_eq!(
        request.correlation().model_execution_id(),
        Some(execution_id)
    );
    assert_eq!(request.idempotency_key().as_str(), "model-request-7");
    assert!(Prompt::new(String::new()).is_err());
    assert!(Prompt::new("x".repeat(64 * 1024 + 1)).is_err());
}

#[test]
fn retry_policy_and_response_correlation_are_explicit() {
    assert!(RetryPolicy::new(0).is_err());
    assert_eq!(
        RetryPolicy::new(2)
            .expect("retry policy is valid")
            .max_attempts(),
        2
    );

    let execution_id = ModelExecutionId::new(8).expect("execution ID is nonzero");
    let output = ModelOutput::new("structured response").expect("output is bounded");
    let response = ModelResponse::new(execution_id, correlation(execution_id), output)
        .expect("matching response correlation is valid");
    assert_eq!(response.execution_id(), execution_id);

    let wrong_execution = ModelExecutionId::new(9).expect("execution ID is nonzero");
    let output = ModelOutput::new("structured response").expect("output is bounded");
    assert!(ModelResponse::new(wrong_execution, correlation(execution_id), output).is_err());
}
