//! Each comparison runs over the lists of every variant the two sides
//! declare, so it reaches each arm of the generated enums and of the
//! conversions, and each match here is exhaustive too, so a variant added on
//! either side fails to compile until this file says what it maps to.

use lablet_model::{
    CacheScope, CompletionMode, ConfigDigest, McpLifetime, ProviderApi, ProviderErrorKind,
    RunContext, RunId, RunLabels, StopReason, ToolCallEnd, ToolCallStatus, ToolSource,
};

use super::super::generated::key;
use super::super::{Attribute, Value};
use super::*;

const fn model_tool_status(registry: LabletToolStatus) -> ToolCallStatus {
    match registry {
        LabletToolStatus::Unknown => ToolCallStatus::Unknown,
        LabletToolStatus::MalformedInput => ToolCallStatus::MalformedInput,
        LabletToolStatus::Rejected => ToolCallStatus::Rejected,
        LabletToolStatus::NotRun => ToolCallStatus::NotRun,
        LabletToolStatus::Ok => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
        LabletToolStatus::ToolError => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError)
        }
        LabletToolStatus::Timeout => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout),
        LabletToolStatus::Failed => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Failed),
        LabletToolStatus::Cancelled => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Cancelled)
        }
    }
}

/// Every status at both levels. A tool that ran is a built-in one, since the
/// flattening drops where it came from.
fn every_tool_call_status() -> impl Iterator<Item = ToolCallStatus> {
    let ran = ToolCallEnd::ALL.map(|ended| ToolCallStatus::ran(ToolSource::Builtin, ended));
    ToolCallStatus::NOTHING_RAN.into_iter().chain(ran)
}

#[test]
fn every_tool_call_status_is_spelled_as_the_registry_spells_it() {
    for status in every_tool_call_status() {
        let registry = LabletToolStatus::from(&status);
        assert_eq!(status.as_str(), registry.as_str());
        assert_eq!(model_tool_status(registry), status);
    }
    for registry in LabletToolStatus::ALL {
        assert_eq!(
            LabletToolStatus::from(&model_tool_status(registry)),
            registry
        );
    }
}

const fn status_with_error(error: LabletExecuteToolErrorType) -> ToolCallStatus {
    match error {
        LabletExecuteToolErrorType::Unknown => ToolCallStatus::Unknown,
        LabletExecuteToolErrorType::MalformedInput => ToolCallStatus::MalformedInput,
        LabletExecuteToolErrorType::Rejected => ToolCallStatus::Rejected,
        LabletExecuteToolErrorType::ToolError => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError)
        }
        LabletExecuteToolErrorType::Timeout => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout)
        }
        LabletExecuteToolErrorType::Failed => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Failed)
        }
        LabletExecuteToolErrorType::Cancelled => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Cancelled)
        }
    }
}

#[test]
fn every_tool_call_status_but_ok_and_not_run_is_the_tool_spans_error_type() {
    for status in every_tool_call_status() {
        let error = tool_error_type(&status);
        if status == ToolCallStatus::NotRun || !status.is_error() {
            assert_eq!(error, None, "{status:?}");
        } else {
            let error = error.unwrap();
            assert_eq!(error.as_str(), status.as_str());
            assert_eq!(status_with_error(error), status);
        }
    }
    for error in LabletExecuteToolErrorType::ALL {
        assert_eq!(tool_error_type(&status_with_error(error)), Some(error));
    }
}

fn model_tool_source(registry: LabletToolSource) -> ToolSource {
    match registry {
        LabletToolSource::Builtin => ToolSource::Builtin,
        LabletToolSource::Mcp => ToolSource::Mcp {
            server: "docs".to_owned(),
        },
    }
}

/// One source of each kind the domain has. An MCP source holds its server's
/// name, so the domain has no list of every source.
fn each_kind_of_source() -> [ToolSource; 2] {
    [
        ToolSource::Builtin,
        ToolSource::Mcp {
            server: "search".to_owned(),
        },
    ]
}

#[test]
fn both_tool_sources_are_spelled_as_the_registry_spells_them() {
    for registry in LabletToolSource::ALL {
        let source = model_tool_source(registry);
        assert_eq!(source.as_str(), registry.as_str());
        assert_eq!(LabletToolSource::from(&source), registry);
    }
    for source in each_kind_of_source() {
        let registry = LabletToolSource::from(&source);
        assert_eq!(registry.as_str(), source.as_str(), "{source:?}");
        assert_eq!(
            std::mem::discriminant(&model_tool_source(registry)),
            std::mem::discriminant(&source),
            "{source:?}"
        );
    }
}

fn source_of_tool_type(tool_type: LabletExecuteToolGenAiToolType) -> ToolSource {
    match tool_type {
        LabletExecuteToolGenAiToolType::Function => ToolSource::Builtin,
        LabletExecuteToolGenAiToolType::Extension => ToolSource::Mcp {
            server: "docs".to_owned(),
        },
    }
}

/// The conventions' spellings, which the golden fixtures carry.
#[test]
fn a_builtin_tool_is_a_function_and_an_mcp_tool_an_extension() {
    for source in each_kind_of_source() {
        let tool_type = LabletExecuteToolGenAiToolType::from(&source);
        let expected = match source {
            ToolSource::Builtin => "function",
            ToolSource::Mcp { .. } => "extension",
        };
        assert_eq!(tool_type.as_str(), expected);
        assert_eq!(
            std::mem::discriminant(&source_of_tool_type(tool_type)),
            std::mem::discriminant(&source)
        );
    }
    for tool_type in LabletExecuteToolGenAiToolType::ALL {
        assert_eq!(
            LabletExecuteToolGenAiToolType::from(&source_of_tool_type(tool_type)),
            tool_type
        );
    }
}

const fn model_transport(registry: LabletExecuteToolNetworkTransport) -> NetworkTransport {
    match registry {
        LabletExecuteToolNetworkTransport::Pipe => NetworkTransport::Pipe,
        LabletExecuteToolNetworkTransport::Tcp => NetworkTransport::Tcp,
    }
}

#[test]
fn both_transports_are_spelled_as_the_conventions_spell_them() {
    for transport in [NetworkTransport::Pipe, NetworkTransport::Tcp] {
        let registry = LabletExecuteToolNetworkTransport::from(transport);
        assert_eq!(registry.as_str(), transport.as_str());
        assert_eq!(model_transport(registry), transport);
    }
    for registry in LabletExecuteToolNetworkTransport::ALL {
        assert_eq!(
            LabletExecuteToolNetworkTransport::from(model_transport(registry)),
            registry
        );
    }
}

/// `None` for the class that isn't a provider error.
const fn model_kind(class: LabletChatErrorType) -> Option<ProviderErrorKind> {
    match class {
        LabletChatErrorType::Retryable => Some(ProviderErrorKind::Retryable),
        LabletChatErrorType::ContextExhausted => Some(ProviderErrorKind::ContextExhausted),
        LabletChatErrorType::Auth => Some(ProviderErrorKind::Auth),
        LabletChatErrorType::Fatal => Some(ProviderErrorKind::Fatal),
        LabletChatErrorType::Malformed => Some(ProviderErrorKind::Malformed),
        LabletChatErrorType::Cancelled => None,
    }
}

#[test]
fn every_provider_error_kind_is_a_class_of_the_chat_spans_error_type_and_cancelled_the_other() {
    for kind in ProviderErrorKind::ALL {
        let class = LabletChatErrorType::from(kind);
        assert_eq!(class.as_str(), kind.as_str());
        assert_eq!(model_kind(class), Some(kind));
    }
    for class in LabletChatErrorType::ALL {
        match model_kind(class) {
            Some(kind) => assert_eq!(LabletChatErrorType::from(kind), class),
            // The run's stop reason names the attempt that was dropped.
            None => assert_eq!(class.as_str(), StopReason::Cancelled.as_str()),
        }
    }
}

const fn model_kind_of_exception(
    class: GenAiClientOperationExceptionExceptionType,
) -> ProviderErrorKind {
    match class {
        GenAiClientOperationExceptionExceptionType::Retryable => ProviderErrorKind::Retryable,
        GenAiClientOperationExceptionExceptionType::ContextExhausted => {
            ProviderErrorKind::ContextExhausted
        }
        GenAiClientOperationExceptionExceptionType::Auth => ProviderErrorKind::Auth,
        GenAiClientOperationExceptionExceptionType::Fatal => ProviderErrorKind::Fatal,
        GenAiClientOperationExceptionExceptionType::Malformed => ProviderErrorKind::Malformed,
    }
}

#[test]
fn every_provider_error_kind_is_a_class_of_the_exception_records_type() {
    for kind in ProviderErrorKind::ALL {
        let class = GenAiClientOperationExceptionExceptionType::from(kind);
        assert_eq!(class.as_str(), kind.as_str());
        assert_eq!(model_kind_of_exception(class), kind);
    }
    for class in GenAiClientOperationExceptionExceptionType::ALL {
        assert_eq!(
            GenAiClientOperationExceptionExceptionType::from(model_kind_of_exception(class)),
            class
        );
    }
}

/// Every value becomes the text it's spelt as, whichever enum it's from.
#[test]
fn each_value_becomes_the_text_it_is_spelt_as() {
    fn each<T: Copy + Into<Value>>(all: &[T], as_str: fn(T) -> &'static str) {
        for value in all {
            assert_eq!((*value).into(), Value::Text(as_str(*value).to_owned()));
        }
    }
    each(&LabletToolStatus::ALL, LabletToolStatus::as_str);
    each(&LabletToolSource::ALL, LabletToolSource::as_str);
    each(
        &LabletExecuteToolErrorType::ALL,
        LabletExecuteToolErrorType::as_str,
    );
    each(
        &LabletExecuteToolGenAiToolType::ALL,
        LabletExecuteToolGenAiToolType::as_str,
    );
    each(
        &LabletExecuteToolNetworkTransport::ALL,
        LabletExecuteToolNetworkTransport::as_str,
    );
    each(&LabletChatErrorType::ALL, LabletChatErrorType::as_str);
    each(
        &GenAiClientOperationExceptionExceptionType::ALL,
        GenAiClientOperationExceptionExceptionType::as_str,
    );
    each(&LabletRunStopReason::ALL, LabletRunStopReason::as_str);
    each(
        &LabletInvokeAgentErrorType::ALL,
        LabletInvokeAgentErrorType::as_str,
    );
    each(&LabletRunErrorType::ALL, LabletRunErrorType::as_str);
    each(
        &LabletRunCompletionMode::ALL,
        LabletRunCompletionMode::as_str,
    );
    each(&LabletRequestApi::ALL, LabletRequestApi::as_str);
    each(
        &LabletRequestCacheScope::ALL,
        LabletRequestCacheScope::as_str,
    );
    each(&LabletMcpLifetime::ALL, LabletMcpLifetime::as_str);
}

fn context(labels: RunLabels) -> RunContext {
    RunContext {
        run_id: RunId::ulid(7),
        labels,
        started: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        config_digest: ConfigDigest::from_sha256([0xab; 32]),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

#[test]
fn the_join_is_the_runs_id_twice_its_config_digest_and_the_labels_it_has() {
    let labelled = context(RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("tool-descriptions-v2".to_owned()),
        trial: Some("seed-42".to_owned()),
    });
    let join = Join::from(&labelled);
    assert_eq!(
        join,
        Join {
            gen_ai_conversation_id: labelled.run_id.as_str().to_owned(),
            lablet_config_digest: labelled.config_digest.as_str().to_owned(),
            session_id: labelled.run_id.as_str().to_owned(),
            lablet_experiment_id: Some("tool-descriptions-v2".to_owned()),
            lablet_task_id: Some("fix-failing-test".to_owned()),
            lablet_trial: Some("seed-42".to_owned()),
        }
    );
    assert_eq!(
        join.attributes(),
        [
            Attribute::of(key::GEN_AI_CONVERSATION_ID, labelled.run_id.as_str()),
            Attribute::of(key::LABLET_CONFIG_DIGEST, labelled.config_digest.as_str()),
            Attribute::of(key::SESSION_ID, labelled.run_id.as_str()),
            Attribute::of(key::LABLET_EXPERIMENT_ID, "tool-descriptions-v2"),
            Attribute::of(key::LABLET_TASK_ID, "fix-failing-test"),
            Attribute::of(key::LABLET_TRIAL, "seed-42"),
        ]
    );

    let unlabelled = Join::from(&context(RunLabels::default()));
    assert_eq!(
        unlabelled.attributes(),
        [
            Attribute::of(key::GEN_AI_CONVERSATION_ID, labelled.run_id.as_str()),
            Attribute::of(key::LABLET_CONFIG_DIGEST, labelled.config_digest.as_str()),
            Attribute::of(key::SESSION_ID, labelled.run_id.as_str()),
            Attribute::new(key::LABLET_EXPERIMENT_ID, None),
            Attribute::new(key::LABLET_TASK_ID, None),
            Attribute::new(key::LABLET_TRIAL, None),
        ]
    );
}

const fn model_stop_reason(registry: LabletRunStopReason) -> StopReason {
    match registry {
        LabletRunStopReason::Completed => StopReason::Completed,
        LabletRunStopReason::EndedWithoutCompletion => StopReason::EndedWithoutCompletion,
        LabletRunStopReason::MaxTurns => StopReason::MaxTurns,
        LabletRunStopReason::Timeout => StopReason::Timeout,
        LabletRunStopReason::MaxTotalTokens => StopReason::MaxTotalTokens,
        LabletRunStopReason::OutputTruncated => StopReason::OutputTruncated,
        LabletRunStopReason::ContextExhausted => StopReason::ContextExhausted,
        LabletRunStopReason::RetriesExhausted => StopReason::RetriesExhausted,
        LabletRunStopReason::InvalidCallsExhausted => StopReason::InvalidCallsExhausted,
        LabletRunStopReason::Cancelled => StopReason::Cancelled,
        LabletRunStopReason::ProviderError => StopReason::ProviderError,
        LabletRunStopReason::Refused => StopReason::Refused,
    }
}

#[test]
fn every_stop_reason_is_spelled_as_the_registry_spells_it() {
    for reason in StopReason::ALL {
        let registry = LabletRunStopReason::from(reason);
        assert_eq!(reason.as_str(), registry.as_str());
        assert_eq!(model_stop_reason(registry), reason);
    }
    for registry in LabletRunStopReason::ALL {
        assert_eq!(
            LabletRunStopReason::from(model_stop_reason(registry)),
            registry
        );
    }
}

const fn stopped_for_root(error: LabletInvokeAgentErrorType) -> StopReason {
    match error {
        LabletInvokeAgentErrorType::EndedWithoutCompletion => StopReason::EndedWithoutCompletion,
        LabletInvokeAgentErrorType::MaxTurns => StopReason::MaxTurns,
        LabletInvokeAgentErrorType::Timeout => StopReason::Timeout,
        LabletInvokeAgentErrorType::MaxTotalTokens => StopReason::MaxTotalTokens,
        LabletInvokeAgentErrorType::OutputTruncated => StopReason::OutputTruncated,
        LabletInvokeAgentErrorType::ContextExhausted => StopReason::ContextExhausted,
        LabletInvokeAgentErrorType::RetriesExhausted => StopReason::RetriesExhausted,
        LabletInvokeAgentErrorType::InvalidCallsExhausted => StopReason::InvalidCallsExhausted,
        LabletInvokeAgentErrorType::Cancelled => StopReason::Cancelled,
        LabletInvokeAgentErrorType::ProviderError => StopReason::ProviderError,
        LabletInvokeAgentErrorType::Refused => StopReason::Refused,
    }
}

const fn stopped_for_wide(error: LabletRunErrorType) -> StopReason {
    match error {
        LabletRunErrorType::EndedWithoutCompletion => StopReason::EndedWithoutCompletion,
        LabletRunErrorType::MaxTurns => StopReason::MaxTurns,
        LabletRunErrorType::Timeout => StopReason::Timeout,
        LabletRunErrorType::MaxTotalTokens => StopReason::MaxTotalTokens,
        LabletRunErrorType::OutputTruncated => StopReason::OutputTruncated,
        LabletRunErrorType::ContextExhausted => StopReason::ContextExhausted,
        LabletRunErrorType::RetriesExhausted => StopReason::RetriesExhausted,
        LabletRunErrorType::InvalidCallsExhausted => StopReason::InvalidCallsExhausted,
        LabletRunErrorType::Cancelled => StopReason::Cancelled,
        LabletRunErrorType::ProviderError => StopReason::ProviderError,
        LabletRunErrorType::Refused => StopReason::Refused,
    }
}

/// Every stop reason but `completed` is an `error.type` on the root span and
/// on the wide event, spelt as the reason is, and `completed` is none.
#[test]
fn every_stop_reason_but_completed_is_the_error_type_of_the_root_span_and_the_wide_event() {
    for reason in StopReason::ALL {
        let (root, wide) = (invoke_agent_error_type(reason), run_error_type(reason));
        if reason == StopReason::Completed {
            assert_eq!(root, None);
            assert_eq!(wide, None);
        } else {
            let root = root.unwrap();
            let wide = wide.unwrap();
            assert_eq!(root.as_str(), reason.as_str());
            assert_eq!(wide.as_str(), reason.as_str());
            assert_eq!(stopped_for_root(root), reason);
            assert_eq!(stopped_for_wide(wide), reason);
        }
    }
    for error in LabletInvokeAgentErrorType::ALL {
        assert_eq!(
            invoke_agent_error_type(stopped_for_root(error)),
            Some(error)
        );
    }
    for error in LabletRunErrorType::ALL {
        assert_eq!(run_error_type(stopped_for_wide(error)), Some(error));
    }
}

const fn model_completion_mode(registry: LabletRunCompletionMode) -> CompletionMode {
    match registry {
        LabletRunCompletionMode::Natural => CompletionMode::Natural,
        LabletRunCompletionMode::Explicit => CompletionMode::Explicit,
    }
}

#[test]
fn both_completion_modes_are_spelled_as_the_registry_spells_them() {
    for mode in CompletionMode::ALL {
        let registry = LabletRunCompletionMode::from(mode);
        assert_eq!(mode.as_str(), registry.as_str());
        assert_eq!(model_completion_mode(registry), mode);
    }
    for registry in LabletRunCompletionMode::ALL {
        assert_eq!(
            LabletRunCompletionMode::from(model_completion_mode(registry)),
            registry
        );
    }
}

const fn model_api(registry: LabletRequestApi) -> ProviderApi {
    match registry {
        LabletRequestApi::Messages => ProviderApi::Messages,
        LabletRequestApi::Responses => ProviderApi::Responses,
        LabletRequestApi::ChatCompletions => ProviderApi::ChatCompletions,
        LabletRequestApi::Script => ProviderApi::Script,
    }
}

#[test]
fn every_api_is_spelled_as_the_registry_spells_it() {
    for api in ProviderApi::ALL {
        let registry = LabletRequestApi::from(api);
        assert_eq!(api.as_str(), registry.as_str());
        assert_eq!(model_api(registry), api);
    }
    for registry in LabletRequestApi::ALL {
        assert_eq!(LabletRequestApi::from(model_api(registry)), registry);
    }
}

const fn model_cache_scope(registry: LabletRequestCacheScope) -> CacheScope {
    match registry {
        LabletRequestCacheScope::Shared => CacheScope::Shared,
        LabletRequestCacheScope::Run => CacheScope::Run,
    }
}

#[test]
fn both_cache_scopes_are_spelled_as_the_registry_spells_them() {
    for scope in CacheScope::ALL {
        let registry = LabletRequestCacheScope::from(scope);
        assert_eq!(scope.as_str(), registry.as_str());
        assert_eq!(model_cache_scope(registry), scope);
    }
    for registry in LabletRequestCacheScope::ALL {
        assert_eq!(
            LabletRequestCacheScope::from(model_cache_scope(registry)),
            registry
        );
    }
}

const fn model_lifetime(registry: LabletMcpLifetime) -> McpLifetime {
    match registry {
        LabletMcpLifetime::Run => McpLifetime::Run,
        LabletMcpLifetime::Lablet => McpLifetime::Lablet,
    }
}

#[test]
fn both_lifetimes_of_an_mcp_server_are_spelled_as_the_registry_spells_them() {
    for lifetime in McpLifetime::ALL {
        let registry = LabletMcpLifetime::from(lifetime);
        assert_eq!(lifetime.as_str(), registry.as_str());
        assert_eq!(model_lifetime(registry), lifetime);
    }
    for registry in LabletMcpLifetime::ALL {
        assert_eq!(LabletMcpLifetime::from(model_lifetime(registry)), registry);
    }
}
