//! The domain can't depend on the generated registry crate, so the two sets of
//! closed values are compared here. Each match is exhaustive, so a variant
//! added on either side fails to compile until the other side has it too.

use lablet_model::{CompletionMode, StopReason, ToolSource};
use lablet_telemetry_registry::enums::{
    LabletRunCompletionMode, LabletRunStopReason, LabletToolSource,
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
    ] {
        let registry = registry_stop_reason(reason);
        assert_eq!(reason.as_str(), registry.as_str());
        assert_eq!(model_stop_reason(registry), reason);
    }
}

#[test]
fn both_completion_modes_are_spelled_as_the_registry_spells_them() {
    let pairs = [
        (CompletionMode::Natural, LabletRunCompletionMode::Natural),
        (CompletionMode::Explicit, LabletRunCompletionMode::Explicit),
    ];
    for (model, registry) in pairs {
        assert_eq!(model.as_str(), registry.as_str());
        let back = match registry {
            LabletRunCompletionMode::Natural => CompletionMode::Natural,
            LabletRunCompletionMode::Explicit => CompletionMode::Explicit,
        };
        assert_eq!(back, model);
    }
}

#[test]
fn both_tool_sources_are_spelled_as_the_registry_spells_them() {
    let mcp = ToolSource::Mcp {
        server: "docs".to_owned(),
    };
    for (model, registry) in [
        (ToolSource::Builtin, LabletToolSource::Builtin),
        (mcp, LabletToolSource::Mcp),
    ] {
        assert_eq!(model.as_str(), registry.as_str());
        let expected = match registry {
            LabletToolSource::Builtin => "builtin",
            LabletToolSource::Mcp => "mcp",
        };
        assert_eq!(model.as_str(), expected);
    }
}
