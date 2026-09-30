//! The domain can't depend on the generated registry crate, so the two sets of
//! closed values are compared here. Each match is exhaustive, so a variant
//! added on either side fails to compile until the other side has it too, and
//! each comparison runs over the lists of every variant the two sides declare,
//! so it reaches that variant once it compiles.

use lablet_model::{
    CacheScope, CompletionMode, McpLifetime, ProviderApi, StopReason, ToolCallEnd, ToolCallStatus,
    ToolSource,
};
use lablet_telemetry_registry::enums::{
    LabletMcpLifetime, LabletRequestApi, LabletRequestCacheScope, LabletRunCompletionMode,
    LabletRunStopReason, LabletToolSource, LabletToolStatus,
};

const fn registry_stop_reason(reason: StopReason) -> LabletRunStopReason {
    match reason {
        StopReason::Completed => LabletRunStopReason::Completed,
        StopReason::EndedWithoutCompletion => LabletRunStopReason::EndedWithoutCompletion,
        StopReason::MaxTurns => LabletRunStopReason::MaxTurns,
        StopReason::Timeout => LabletRunStopReason::Timeout,
        StopReason::MaxTotalTokens => LabletRunStopReason::MaxTotalTokens,
        StopReason::OutputTruncated => LabletRunStopReason::OutputTruncated,
        StopReason::ContextExhausted => LabletRunStopReason::ContextExhausted,
        StopReason::RetriesExhausted => LabletRunStopReason::RetriesExhausted,
        StopReason::InvalidCallsExhausted => LabletRunStopReason::InvalidCallsExhausted,
        StopReason::Cancelled => LabletRunStopReason::Cancelled,
        StopReason::ProviderError => LabletRunStopReason::ProviderError,
        StopReason::Refused => LabletRunStopReason::Refused,
    }
}

const fn model_stop_reason(reason: LabletRunStopReason) -> StopReason {
    match reason {
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
        let registry = registry_stop_reason(reason);
        assert_eq!(reason.as_str(), registry.as_str());
        assert_eq!(model_stop_reason(registry), reason);
    }
    for registry in LabletRunStopReason::ALL {
        assert_eq!(registry_stop_reason(model_stop_reason(registry)), registry);
    }
}

const fn registry_completion_mode(mode: CompletionMode) -> LabletRunCompletionMode {
    match mode {
        CompletionMode::Natural => LabletRunCompletionMode::Natural,
        CompletionMode::Explicit => LabletRunCompletionMode::Explicit,
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
        let registry = registry_completion_mode(mode);
        assert_eq!(mode.as_str(), registry.as_str());
        assert_eq!(model_completion_mode(registry), mode);
    }
    for registry in LabletRunCompletionMode::ALL {
        assert_eq!(
            registry_completion_mode(model_completion_mode(registry)),
            registry
        );
    }
}

const fn registry_tool_source(source: &ToolSource) -> LabletToolSource {
    match source {
        ToolSource::Builtin => LabletToolSource::Builtin,
        ToolSource::Mcp { .. } => LabletToolSource::Mcp,
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

/// An MCP source holds its server's name, so the domain has no list of every
/// source, and the registry's list reaches each kind of source through the
/// match above.
#[test]
fn both_tool_sources_are_spelled_as_the_registry_spells_them() {
    for registry in LabletToolSource::ALL {
        let source = model_tool_source(registry);
        assert_eq!(source.as_str(), registry.as_str());
        assert_eq!(registry_tool_source(&source), registry);
    }
    // The other way round, from one source of each kind the domain has, so a
    // kind the domain gains and maps onto another's spelling fails here. A
    // kind added to the domain doesn't compile until `registry_tool_source`
    // matches it, beside this list.
    let each_kind = [
        ToolSource::Builtin,
        ToolSource::Mcp {
            server: "search".to_owned(),
        },
    ];
    for source in each_kind {
        let registry = registry_tool_source(&source);
        assert_eq!(registry.as_str(), source.as_str(), "{source:?}");
        assert_eq!(
            std::mem::discriminant(&model_tool_source(registry)),
            std::mem::discriminant(&source),
            "{source:?}"
        );
    }
}

/// The model nests the registry's values in two levels, so both levels are
/// matched exhaustively here and the flattening is what's compared.
const fn registry_tool_status(status: &ToolCallStatus) -> LabletToolStatus {
    match status {
        ToolCallStatus::Unknown => LabletToolStatus::Unknown,
        ToolCallStatus::MalformedInput => LabletToolStatus::MalformedInput,
        ToolCallStatus::Rejected => LabletToolStatus::Rejected,
        ToolCallStatus::NotRun => LabletToolStatus::NotRun,
        ToolCallStatus::Ran { ended, .. } => match ended {
            ToolCallEnd::Ok => LabletToolStatus::Ok,
            ToolCallEnd::ToolError => LabletToolStatus::ToolError,
            ToolCallEnd::Timeout => LabletToolStatus::Timeout,
            ToolCallEnd::Failed => LabletToolStatus::Failed,
            ToolCallEnd::Cancelled => LabletToolStatus::Cancelled,
        },
    }
}

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
        let registry = registry_tool_status(&status);
        assert_eq!(status.as_str(), registry.as_str());
        assert_eq!(model_tool_status(registry), status);
    }
    for registry in LabletToolStatus::ALL {
        assert_eq!(registry_tool_status(&model_tool_status(registry)), registry);
    }
}

const fn registry_api(api: ProviderApi) -> LabletRequestApi {
    match api {
        ProviderApi::Messages => LabletRequestApi::Messages,
        ProviderApi::Responses => LabletRequestApi::Responses,
        ProviderApi::ChatCompletions => LabletRequestApi::ChatCompletions,
        ProviderApi::Script => LabletRequestApi::Script,
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
        let registry = registry_api(api);
        assert_eq!(api.as_str(), registry.as_str());
        assert_eq!(model_api(registry), api);
    }
    for registry in LabletRequestApi::ALL {
        assert_eq!(registry_api(model_api(registry)), registry);
    }
}

const fn registry_cache_scope(scope: CacheScope) -> LabletRequestCacheScope {
    match scope {
        CacheScope::Shared => LabletRequestCacheScope::Shared,
        CacheScope::Run => LabletRequestCacheScope::Run,
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
        let registry = registry_cache_scope(scope);
        assert_eq!(scope.as_str(), registry.as_str());
        assert_eq!(model_cache_scope(registry), scope);
    }
    for registry in LabletRequestCacheScope::ALL {
        assert_eq!(registry_cache_scope(model_cache_scope(registry)), registry);
    }
}

const fn registry_lifetime(lifetime: McpLifetime) -> LabletMcpLifetime {
    match lifetime {
        McpLifetime::Run => LabletMcpLifetime::Run,
        McpLifetime::Lablet => LabletMcpLifetime::Lablet,
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
        let registry = registry_lifetime(lifetime);
        assert_eq!(lifetime.as_str(), registry.as_str());
        assert_eq!(model_lifetime(registry), lifetime);
    }
    for registry in LabletMcpLifetime::ALL {
        assert_eq!(registry_lifetime(model_lifetime(registry)), registry);
    }
}
