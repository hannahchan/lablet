use super::*;

use StopPoint::{AfterProviderResponse, AfterToolPhase, BeforeProviderCall};

const TIMEOUT: Duration = Duration::from_secs(600);

/// Limits far from anything the states below reach.
fn policy(completion: CompletionMode) -> StopPolicy {
    StopPolicy {
        completion,
        max_turns: 30,
        timeout: TIMEOUT,
        max_total_tokens: None,
        max_consecutive_tool_errors: 3,
    }
}

fn natural() -> StopPolicy {
    policy(CompletionMode::Natural)
}

fn explicit() -> StopPolicy {
    policy(CompletionMode::Explicit)
}

/// The first turn of a run, after a response that called a tool.
fn mid_run() -> RunState {
    RunState {
        turn: 1,
        elapsed: Duration::from_secs(1),
        total_tokens: 100,
        consecutive_tool_errors: 0,
        last_finish: Some(FinishReason::ToolUse),
        last_had_tool_use: true,
        task_complete_called: false,
    }
}

/// A response with no tool call and the given finish reason.
fn answered(finish: FinishReason) -> RunState {
    RunState {
        last_finish: Some(finish),
        last_had_tool_use: false,
        ..mid_run()
    }
}

/// A state in which every limit and cap of `policy` is reached at once.
fn everything_reached(policy: &StopPolicy) -> RunState {
    RunState {
        turn: policy.max_turns,
        elapsed: policy.timeout,
        total_tokens: policy.max_total_tokens.unwrap(),
        consecutive_tool_errors: policy.max_consecutive_tool_errors,
        ..mid_run()
    }
}

#[test]
fn a_run_inside_every_limit_goes_on_at_every_point() {
    for at in [BeforeProviderCall, AfterProviderResponse, AfterToolPhase] {
        assert_eq!(natural().evaluate(at, &mid_run()), None, "{at:?}");
        assert_eq!(explicit().evaluate(at, &mid_run()), None, "{at:?}");
    }
}

#[test]
fn a_fresh_run_makes_its_first_provider_call() {
    let state = RunState {
        turn: 1,
        ..RunState::default()
    };

    assert_eq!(natural().evaluate(BeforeProviderCall, &state), None);
}

// The turn cap

#[test]
fn the_turn_cap_stops_the_run_after_the_tool_phase_of_the_capped_turn() {
    let policy = StopPolicy {
        max_turns: 2,
        ..natural()
    };
    let turn = |turn| RunState { turn, ..mid_run() };

    assert_eq!(policy.evaluate(AfterToolPhase, &turn(1)), None);
    assert_eq!(
        policy.evaluate(AfterToolPhase, &turn(2)),
        Some(StopReason::MaxTurns)
    );
    assert_eq!(
        policy.evaluate(AfterToolPhase, &turn(3)),
        Some(StopReason::MaxTurns)
    );
}

#[test]
fn the_turn_cap_is_not_read_before_a_provider_call_or_after_a_response() {
    let policy = StopPolicy {
        max_turns: 2,
        ..natural()
    };
    let state = RunState {
        turn: 2,
        ..mid_run()
    };

    assert_eq!(policy.evaluate(BeforeProviderCall, &state), None);
    assert_eq!(policy.evaluate(AfterProviderResponse, &state), None);
}

#[test]
fn a_response_that_finishes_on_the_capped_turn_completes_the_run() {
    let policy = StopPolicy {
        max_turns: 2,
        ..natural()
    };
    let state = RunState {
        turn: 2,
        ..answered(FinishReason::EndTurn)
    };

    assert_eq!(
        policy.evaluate(AfterProviderResponse, &state),
        Some(StopReason::Completed)
    );
}

#[test]
fn a_turn_cap_of_zero_acts_as_one() {
    let policy = StopPolicy {
        max_turns: 0,
        ..natural()
    };

    assert_eq!(policy.evaluate(BeforeProviderCall, &mid_run()), None);
    assert_eq!(
        policy.evaluate(AfterToolPhase, &mid_run()),
        Some(StopReason::MaxTurns)
    );
}

// The timeout

#[test]
fn the_timeout_stops_the_run_at_the_instant_it_is_reached() {
    let nanosecond = Duration::from_nanos(1);
    let at_elapsed = |elapsed| RunState {
        elapsed,
        ..mid_run()
    };

    for at in [BeforeProviderCall, AfterToolPhase] {
        assert_eq!(
            natural().evaluate(at, &at_elapsed(TIMEOUT.checked_sub(nanosecond).unwrap())),
            None,
            "{at:?}"
        );
        assert_eq!(
            natural().evaluate(at, &at_elapsed(TIMEOUT)),
            Some(StopReason::Timeout),
            "{at:?}"
        );
        assert_eq!(
            natural().evaluate(at, &at_elapsed(TIMEOUT + nanosecond)),
            Some(StopReason::Timeout),
            "{at:?}"
        );
    }
}

#[test]
fn a_timeout_of_zero_stops_the_run_before_its_first_provider_call() {
    let policy = StopPolicy {
        timeout: Duration::ZERO,
        ..natural()
    };
    let state = RunState {
        turn: 1,
        ..RunState::default()
    };

    assert_eq!(
        policy.evaluate(BeforeProviderCall, &state),
        Some(StopReason::Timeout)
    );
}

// The token budget

#[test]
fn the_token_budget_stops_the_run_on_the_token_that_reaches_it() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..natural()
    };
    let with_tokens = |total_tokens| RunState {
        total_tokens,
        ..mid_run()
    };

    for at in [BeforeProviderCall, AfterToolPhase] {
        assert_eq!(policy.evaluate(at, &with_tokens(999)), None, "{at:?}");
        assert_eq!(
            policy.evaluate(at, &with_tokens(1_000)),
            Some(StopReason::MaxTotalTokens),
            "{at:?}"
        );
        assert_eq!(
            policy.evaluate(at, &with_tokens(1_001)),
            Some(StopReason::MaxTotalTokens),
            "{at:?}"
        );
    }
}

#[test]
fn a_run_without_a_token_budget_is_never_stopped_for_tokens() {
    let state = RunState {
        total_tokens: u64::MAX,
        ..mid_run()
    };

    assert_eq!(natural().max_total_tokens, None);
    assert_eq!(natural().evaluate(BeforeProviderCall, &state), None);
    assert_eq!(natural().evaluate(AfterToolPhase, &state), None);
}

// The consecutive tool-error cap

#[test]
fn the_tool_error_cap_stops_the_run_on_the_error_that_reaches_it() {
    let after_errors = |consecutive_tool_errors| RunState {
        consecutive_tool_errors,
        ..mid_run()
    };

    assert_eq!(natural().max_consecutive_tool_errors, 3);
    assert_eq!(natural().evaluate(AfterToolPhase, &after_errors(2)), None);
    assert_eq!(
        natural().evaluate(AfterToolPhase, &after_errors(3)),
        Some(StopReason::ToolErrorsExhausted)
    );
    assert_eq!(
        natural().evaluate(AfterToolPhase, &after_errors(4)),
        Some(StopReason::ToolErrorsExhausted)
    );
}

#[test]
fn the_tool_error_cap_is_not_read_before_a_provider_call_or_after_a_response() {
    let state = RunState {
        consecutive_tool_errors: 3,
        ..mid_run()
    };

    assert_eq!(natural().evaluate(BeforeProviderCall, &state), None);
    assert_eq!(natural().evaluate(AfterProviderResponse, &state), None);
}

#[test]
fn a_tool_error_cap_of_zero_acts_as_one() {
    let policy = StopPolicy {
        max_consecutive_tool_errors: 0,
        ..natural()
    };
    let after_errors = |consecutive_tool_errors| RunState {
        consecutive_tool_errors,
        ..mid_run()
    };

    assert_eq!(policy.evaluate(AfterToolPhase, &after_errors(0)), None);
    assert_eq!(
        policy.evaluate(AfterToolPhase, &after_errors(1)),
        Some(StopReason::ToolErrorsExhausted)
    );
}

// The response, in natural mode

#[test]
fn natural_mode_completes_on_a_response_with_no_tool_calls() {
    for finish in [
        FinishReason::EndTurn,
        FinishReason::Other("stop_sequence".to_owned()),
    ] {
        assert_eq!(
            natural().evaluate(AfterProviderResponse, &answered(finish.clone())),
            Some(StopReason::Completed),
            "{finish}"
        );
    }
}

#[test]
fn natural_mode_goes_on_when_the_response_has_tool_calls() {
    assert_eq!(natural().evaluate(AfterProviderResponse, &mid_run()), None);
}

#[test]
fn natural_mode_does_not_read_task_complete() {
    let state = RunState {
        task_complete_called: true,
        ..mid_run()
    };

    assert_eq!(natural().evaluate(AfterProviderResponse, &state), None);
}

// The response, in explicit mode

#[test]
fn explicit_mode_completes_when_task_complete_is_called() {
    let state = RunState {
        task_complete_called: true,
        ..mid_run()
    };

    assert_eq!(
        explicit().evaluate(AfterProviderResponse, &state),
        Some(StopReason::Completed)
    );
}

#[test]
fn explicit_mode_ends_without_completion_on_a_response_with_no_tool_calls() {
    assert_eq!(
        explicit().evaluate(AfterProviderResponse, &answered(FinishReason::EndTurn)),
        Some(StopReason::EndedWithoutCompletion)
    );
}

#[test]
fn explicit_mode_goes_on_when_the_response_calls_other_tools() {
    assert_eq!(explicit().evaluate(AfterProviderResponse, &mid_run()), None);
}

// A truncated response

#[test]
fn a_truncated_response_with_no_tool_calls_is_output_truncated_in_both_modes() {
    let state = answered(FinishReason::MaxTokens);

    assert_eq!(
        natural().evaluate(AfterProviderResponse, &state),
        Some(StopReason::OutputTruncated)
    );
    assert_eq!(
        explicit().evaluate(AfterProviderResponse, &state),
        Some(StopReason::OutputTruncated)
    );
}

#[test]
fn a_truncated_response_with_tool_calls_goes_on_in_both_modes() {
    let state = RunState {
        last_finish: Some(FinishReason::MaxTokens),
        ..mid_run()
    };

    assert_eq!(natural().evaluate(AfterProviderResponse, &state), None);
    assert_eq!(explicit().evaluate(AfterProviderResponse, &state), None);
}

#[test]
fn a_truncated_response_that_called_task_complete_is_output_truncated_not_completed() {
    let state = RunState {
        last_finish: Some(FinishReason::MaxTokens),
        last_had_tool_use: true,
        task_complete_called: true,
        ..mid_run()
    };

    assert_eq!(
        explicit().evaluate(AfterProviderResponse, &state),
        Some(StopReason::OutputTruncated)
    );
    let whole = RunState {
        last_finish: Some(FinishReason::ToolUse),
        ..state
    };
    assert_eq!(
        explicit().evaluate(AfterProviderResponse, &whole),
        Some(StopReason::Completed)
    );
}

#[test]
fn the_finish_reason_is_only_read_after_a_response() {
    let state = RunState {
        last_finish: Some(FinishReason::MaxTokens),
        ..mid_run()
    };

    assert_eq!(natural().evaluate(AfterToolPhase, &state), None);
    assert_eq!(natural().evaluate(BeforeProviderCall, &state), None);
}

// Precedence

#[test]
fn a_response_is_judged_on_its_content_even_when_every_limit_is_reached() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..natural()
    };
    let with_tool_calls = everything_reached(&policy);
    let finished = RunState {
        last_finish: Some(FinishReason::EndTurn),
        last_had_tool_use: false,
        ..everything_reached(&policy)
    };

    assert_eq!(
        policy.evaluate(AfterProviderResponse, &with_tool_calls),
        None
    );
    assert_eq!(
        policy.evaluate(AfterProviderResponse, &finished),
        Some(StopReason::Completed)
    );
}

#[test]
fn before_a_provider_call_the_timeout_is_reported_ahead_of_the_token_budget() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..natural()
    };

    assert_eq!(
        policy.evaluate(BeforeProviderCall, &everything_reached(&policy)),
        Some(StopReason::Timeout)
    );
}

#[test]
fn after_the_tool_phase_the_reasons_rank_tool_errors_then_turns_then_timeout_then_tokens() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..natural()
    };
    let all = everything_reached(&policy);
    let without_errors = RunState {
        consecutive_tool_errors: 0,
        ..all.clone()
    };
    let without_turns = RunState {
        turn: 1,
        ..without_errors.clone()
    };
    let without_timeout = RunState {
        elapsed: Duration::ZERO,
        ..without_turns.clone()
    };

    assert_eq!(
        policy.evaluate(AfterToolPhase, &all),
        Some(StopReason::ToolErrorsExhausted)
    );
    assert_eq!(
        policy.evaluate(AfterToolPhase, &without_errors),
        Some(StopReason::MaxTurns)
    );
    assert_eq!(
        policy.evaluate(AfterToolPhase, &without_turns),
        Some(StopReason::Timeout)
    );
    assert_eq!(
        policy.evaluate(AfterToolPhase, &without_timeout),
        Some(StopReason::MaxTotalTokens)
    );
}
