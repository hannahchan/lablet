//! The domain can't depend on the generated registry crate, so the two sets of
//! closed values are compared here. Each match is exhaustive, so a variant
//! added on either side fails to compile until the other side has it too.

use lablet_model::{CompletionMode, StopReason, ToolCallEnd, ToolCallStatus, ToolSource};
use lablet_telemetry_registry::enums::{
    LabletRunCompletionMode, LabletRunStopReason, LabletToolSource, LabletToolStatus,
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
        StopReason::ToolErrorsExhausted => LabletRunStopReason::ToolErrorsExhausted,
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
        LabletRunStopReason::ToolErrorsExhausted => StopReason::ToolErrorsExhausted,
        LabletRunStopReason::Cancelled => StopReason::Cancelled,
        LabletRunStopReason::ProviderError => StopReason::ProviderError,
        LabletRunStopReason::Refused => StopReason::Refused,
    }
}

#[test]
fn every_stop_reason_is_spelled_as_the_registry_spells_it() {
    for reason in [
        StopReason::Completed,
        StopReason::EndedWithoutCompletion,
        StopReason::MaxTurns,
        StopReason::Timeout,
        StopReason::MaxTotalTokens,
        StopReason::OutputTruncated,
        StopReason::ContextExhausted,
        StopReason::RetriesExhausted,
        StopReason::ToolErrorsExhausted,
        StopReason::Cancelled,
        StopReason::ProviderError,
        StopReason::Refused,
    ] {
        let registry = registry_stop_reason(reason);
        assert_eq!(reason.as_str(), registry.as_str());
        assert_eq!(model_stop_reason(registry), reason);
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
    for mode in [CompletionMode::Natural, CompletionMode::Explicit] {
        let registry = registry_completion_mode(mode);
        assert_eq!(mode.as_str(), registry.as_str());
        assert_eq!(model_completion_mode(registry), mode);
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

#[test]
fn both_tool_sources_are_spelled_as_the_registry_spells_them() {
    for source in [
        ToolSource::Builtin,
        ToolSource::Mcp {
            server: "docs".to_owned(),
        },
    ] {
        let registry = registry_tool_source(&source);
        assert_eq!(source.as_str(), registry.as_str());
        assert_eq!(model_tool_source(registry), source);
    }
}

/// The model nests the five registry values in two levels, so both levels are
/// matched exhaustively here and the flattening is what's compared.
const fn registry_tool_status(status: &ToolCallStatus) -> LabletToolStatus {
    match status {
        ToolCallStatus::Unknown => LabletToolStatus::Unknown,
        ToolCallStatus::Ran { ended, .. } => match ended {
            ToolCallEnd::Ok => LabletToolStatus::Ok,
            ToolCallEnd::ToolError => LabletToolStatus::ToolError,
            ToolCallEnd::Timeout => LabletToolStatus::Timeout,
            ToolCallEnd::Failed => LabletToolStatus::Failed,
        },
    }
}

const fn model_tool_status(registry: LabletToolStatus) -> ToolCallStatus {
    match registry {
        LabletToolStatus::Unknown => ToolCallStatus::Unknown,
        LabletToolStatus::Ok => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
        LabletToolStatus::ToolError => {
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError)
        }
        LabletToolStatus::Timeout => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout),
        LabletToolStatus::Failed => ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Failed),
    }
}

#[test]
fn every_tool_call_status_is_spelled_as_the_registry_spells_it() {
    for status in [
        ToolCallStatus::Unknown,
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError),
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Timeout),
        ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Failed),
    ] {
        let registry = registry_tool_status(&status);
        assert_eq!(status.as_str(), registry.as_str());
        assert_eq!(model_tool_status(registry), status);
    }
}
