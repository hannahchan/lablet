//! Stands in for `lablet-run`: one failed provider attempt, instrumented the
//! way the loop would be, through the generated types only.

pub mod telemetry;

use std::time::{Duration, SystemTime};

use opentelemetry::Context;
use opentelemetry::global::BoxedTracer;
use opentelemetry::trace::{Span as _, TraceContextExt as _, Tracer as _};

use telemetry::{
    ErrorType, GenAiClientOperationException, GenAiOperationName, GenAiProviderName, LabletChat,
    LabletChatPurpose, LabletRetry,
};

/// What the loop measured for one attempt that failed and will be retried.
pub struct FailedAttempt {
    pub run_id: String,
    pub digest: String,
    pub model: String,
    pub turn: i64,
    pub attempt: i64,
    pub started: SystemTime,
    pub latency: Duration,
    pub backoff: Duration,
    pub message: String,
}

/// Records the attempt's span, its failure record and its retry event.
pub fn failed_attempt(tracer: &BoxedTracer, parent: &Context, failed: &FailedAttempt) {
    let chat = LabletChat {
        gen_ai_conversation_id: failed.run_id.clone(),
        gen_ai_operation_name: GenAiOperationName::Chat,
        gen_ai_provider_name: GenAiProviderName::Anthropic,
        gen_ai_request_max_tokens: 1024,
        gen_ai_request_model: failed.model.clone(),
        lablet_attempt: failed.attempt,
        lablet_chat_purpose: LabletChatPurpose::Turn,
        lablet_config_digest: failed.digest.clone(),
        lablet_request_bytes: 512,
        lablet_turn: failed.turn,
        session_id: failed.run_id.clone(),
        error_type: Some(ErrorType::Other("retryable".to_owned())),
        gen_ai_request_reasoning_level: None,
        gen_ai_request_seed: None,
        gen_ai_request_temperature: None,
        gen_ai_response_finish_reasons: None,
        gen_ai_response_id: None,
        gen_ai_response_model: None,
        gen_ai_usage_cache_read_input_tokens: None,
        gen_ai_usage_cache_write_input_tokens: None,
        gen_ai_usage_input_tokens: None,
        gen_ai_usage_output_tokens: None,
        gen_ai_usage_reasoning_output_tokens: None,
        lablet_experiment_id: None,
        lablet_task_id: None,
        lablet_trial: None,
        server_address: None,
        server_port: None,
    };
    let mut span = tracer
        .span_builder(chat.name())
        .with_kind(LabletChat::KIND)
        .with_start_time(failed.started)
        .start_with_context(tracer, parent);
    let ended = failed.started + failed.latency;
    chat.record(&mut span);
    let cx = parent.with_remote_span_context(span.span_context().clone());
    {
        let _attached = cx.attach();
        GenAiClientOperationException {
            exception_message: failed.message.clone(),
            exception_type: "retryable".to_owned(),
            gen_ai_conversation_id: failed.run_id.clone(),
            gen_ai_operation_name: GenAiOperationName::Chat,
            gen_ai_provider_name: GenAiProviderName::Anthropic,
            gen_ai_request_model: failed.model.clone(),
            lablet_attempt: failed.attempt,
            lablet_config_digest: failed.digest.clone(),
            lablet_turn: failed.turn,
            session_id: failed.run_id.clone(),
            lablet_experiment_id: None,
            lablet_task_id: None,
            lablet_trial: Some("3".to_owned()),
        }
        .emit();
    }
    LabletRetry {
        lablet_attempt: failed.attempt,
        lablet_retry_will_retry: true,
        lablet_retry_backoff_ms: Some(i64::try_from(failed.backoff.as_millis()).unwrap_or(i64::MAX)),
    }
    .add_to(&mut span, ended);
    span.end_with_timestamp(ended);
}
