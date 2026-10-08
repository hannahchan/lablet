use lablet_config::ConfigError;
use lablet_model::{OutcomeParts, RunId, RunLabels, RunOutcome, StopReason, TaskResult, Usage};
use lablet_prepare::{BuildError, Unsupported};

use super::{Refusal, of_run};

#[test]
fn a_refused_config_is_a_config_message() {
    let refusal = Refusal::from(ConfigError::Missing {
        key: "model.script".to_owned(),
        reason: "the provider `fake` plays it".to_owned(),
    });
    assert_eq!(
        refusal.to_string(),
        "config: model.script isn't set: the provider `fake` plays it"
    );
}

#[test]
fn a_build_that_failed_is_a_config_message_whatever_failed() {
    let refusals = [
        BuildError::Config(ConfigError::UnknownFormat {
            path: "lablet.toml".to_owned(),
        }),
        BuildError::KeyVariable {
            place: None,
            reason: "`ANTHROPIC_API_KEY` isn't set".to_owned(),
        },
        BuildError::Unsupported {
            kind: Unsupported::Anthropic,
            phase: "7",
        },
        BuildError::Tools {
            reason: "two tools are named bash".to_owned(),
        },
    ];
    for error in refusals {
        let shown = error.to_string();
        assert_eq!(Refusal::from(error).to_string(), format!("config: {shown}"));
    }
}

#[test]
fn what_the_command_line_refuses_itself_is_a_config_message() {
    assert_eq!(
        Refusal::config("the task prompt from --prompt is blank").to_string(),
        "config: the task prompt from --prompt is blank"
    );
}

fn outcome(stop_reason: StopReason, error: Option<&str>) -> RunOutcome {
    RunOutcome::closing(OutcomeParts {
        run_id: RunId::new("r").unwrap(),
        labels: RunLabels::default(),
        stop_reason,
        turns: 0,
        usage: Usage::default(),
        tool_calls: 0,
        duration_ms: 3,
        result: TaskResult {
            text: String::new(),
            structured: None,
        },
        error: error.map(str::to_owned),
    })
}

#[test]
fn a_run_the_provider_s_error_ended_prints_that_error_as_a_provider_message() {
    assert_eq!(
        of_run(&outcome(
            StopReason::ProviderError,
            Some("401 invalid x-api-key")
        )),
        Some("provider: 401 invalid x-api-key".to_owned())
    );
}

#[test]
fn a_run_that_ended_any_other_way_prints_no_message_beside_its_outcome() {
    for (reason, error) in [
        (StopReason::Completed, None),
        (StopReason::Cancelled, None),
        (StopReason::RetriesExhausted, Some("429 slow down")),
        (StopReason::Timeout, None),
    ] {
        assert_eq!(of_run(&outcome(reason, error)), None, "{reason}");
    }
}
