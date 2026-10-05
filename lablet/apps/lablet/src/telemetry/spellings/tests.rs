//! Each comparison runs over the lists of every variant the two sides
//! declare, so it reaches each arm of the generated enums and of the
//! conversions, and each match here is exhaustive too, so a variant added on
//! either side fails to compile until this file says what it maps to.

use super::*;

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
