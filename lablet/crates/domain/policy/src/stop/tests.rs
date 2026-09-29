use lablet_model::{TokenCounts, Usage};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(600);

/// A cap of `count`, which is above zero in every test here: zero would be
/// no cap.
const fn cap(count: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(count)
}

/// Limits far from anything the progress below reaches.
fn limits() -> StopPolicy {
    StopPolicy {
        max_turns: cap(30),
        timeout: TIMEOUT,
        max_total_tokens: None,
        max_consecutive_invalid_turns: cap(3),
    }
}

/// No cap and no budget, so only the timeout is left to stop a run.
fn uncapped() -> StopPolicy {
    StopPolicy {
        max_turns: None,
        timeout: TIMEOUT,
        max_total_tokens: None,
        max_consecutive_invalid_turns: None,
    }
}

const EVERY_MODE: [CompletionMode; 2] = [CompletionMode::Natural, CompletionMode::Explicit];

fn natural(finish: &FinishReason, calls: Calls) -> Option<StopReason> {
    limits().after_response(finish, CompletionMode::Natural, calls)
}

fn explicit(finish: &FinishReason, calls: Calls) -> Option<StopReason> {
    limits().after_response(finish, CompletionMode::Explicit, calls)
}

/// The reason a response that called no tool stops a run in `mode`.
fn final_in(mode: CompletionMode, finish: &FinishReason) -> StopReason {
    limits().after_final(finish, mode)
}

/// One response in, one second gone, 100 tokens used.
fn mid_run() -> Progress {
    Progress {
        turns: 1,
        elapsed: Duration::from_secs(1),
        usage: tokens(100),
        consecutive_invalid_turns: 0,
    }
}

/// Usage whose total is `total`, split so that only a sum of input and output
/// gives it.
const fn tokens(total: u64) -> Usage {
    Usage::from_inclusive(TokenCounts {
        input: total - total / 4,
        output: total / 4,
        reasoning: Some(0),
        cache_read: Some(total / 2),
        cache_write: Some(0),
    })
}

/// Progress at which every limit and cap of `policy` is reached at once.
fn everything_reached(policy: &StopPolicy) -> Progress {
    Progress {
        turns: policy.max_turns.unwrap().get(),
        elapsed: policy.timeout,
        usage: tokens(policy.max_total_tokens.unwrap()),
        consecutive_invalid_turns: policy.max_consecutive_invalid_turns.unwrap().get(),
    }
}

#[test]
fn a_run_inside_every_limit_goes_on_at_every_point() {
    for mode in EVERY_MODE {
        assert_eq!(limits().before_call(&mid_run()), None);
        assert_eq!(
            limits().after_response(&FinishReason::ToolUse, mode, Calls::Tools),
            None
        );
        assert_eq!(limits().after_tools(&mid_run()), None);
    }
}

#[test]
fn a_fresh_run_makes_its_first_provider_call() {
    assert_eq!(limits().before_call(&Progress::default()), None);
}

// The turn cap

#[test]
fn the_turn_cap_stops_the_run_after_the_tool_phase_of_the_capped_turn() {
    let policy = StopPolicy {
        max_turns: cap(2),
        ..limits()
    };
    let after = |turns| Progress { turns, ..mid_run() };

    assert_eq!(policy.after_tools(&after(1)), None);
    assert_eq!(policy.after_tools(&after(2)), Some(StopReason::MaxTurns));
    assert_eq!(policy.after_tools(&after(3)), Some(StopReason::MaxTurns));
}

#[test]
fn the_turn_cap_is_not_read_before_a_provider_call() {
    let policy = StopPolicy {
        max_turns: cap(2),
        ..limits()
    };
    let progress = Progress {
        turns: 2,
        ..mid_run()
    };

    assert_eq!(policy.before_call(&progress), None);
}

#[test]
fn the_smallest_turn_cap_stops_the_run_after_one_turn() {
    let policy = StopPolicy {
        max_turns: cap(1),
        ..limits()
    };

    assert_eq!(policy.before_call(&Progress::default()), None);
    assert_eq!(policy.after_tools(&mid_run()), Some(StopReason::MaxTurns));
}

#[test]
fn a_run_without_a_turn_cap_is_never_stopped_for_its_turns() {
    let policy = StopPolicy {
        max_turns: None,
        ..limits()
    };
    let after = |turns| Progress { turns, ..mid_run() };

    for turns in [1, 30, 41, u32::MAX] {
        assert_eq!(policy.after_tools(&after(turns)), None, "{turns}");
        assert_eq!(policy.before_call(&after(turns)), None, "{turns}");
    }
}

// The timeout

#[test]
fn the_timeout_stops_the_run_at_the_instant_it_is_reached() {
    let nanosecond = Duration::from_nanos(1);
    let at = |elapsed| Progress {
        elapsed,
        ..mid_run()
    };

    for decide in [StopPolicy::before_call, StopPolicy::after_tools] {
        assert_eq!(
            decide(&limits(), &at(TIMEOUT.checked_sub(nanosecond).unwrap())),
            None
        );
        assert_eq!(decide(&limits(), &at(TIMEOUT)), Some(StopReason::Timeout));
        assert_eq!(
            decide(&limits(), &at(TIMEOUT + nanosecond)),
            Some(StopReason::Timeout)
        );
    }
}

#[test]
fn a_timeout_of_zero_stops_the_run_before_its_first_provider_call() {
    let policy = StopPolicy {
        timeout: Duration::ZERO,
        ..limits()
    };

    assert_eq!(
        policy.before_call(&Progress::default()),
        Some(StopReason::Timeout)
    );
}

// The time left

#[test]
fn the_time_left_is_the_timeout_less_what_has_elapsed() {
    assert_eq!(limits().time_left(Duration::ZERO), Some(TIMEOUT));
    assert_eq!(
        limits().time_left(Duration::from_secs(599)),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        limits().time_left(TIMEOUT.checked_sub(Duration::from_nanos(1)).unwrap()),
        Some(Duration::from_nanos(1))
    );
}

#[test]
fn a_run_that_has_reached_its_timeout_has_no_time_left() {
    assert_eq!(limits().time_left(TIMEOUT), None);
    assert_eq!(limits().time_left(TIMEOUT + Duration::from_nanos(1)), None);
    assert_eq!(limits().time_left(Duration::MAX), None);

    let policy = StopPolicy {
        timeout: Duration::ZERO,
        ..limits()
    };
    assert_eq!(policy.time_left(Duration::ZERO), None);
}

/// A deadline is handed out only by a run the timeout hasn't stopped, so the
/// two answers change at the same instant: a run point A or point B lets go
/// on has time to give a call, and one they stop has none.
#[test]
fn a_run_has_time_left_exactly_when_the_timeout_has_not_stopped_it() {
    let nanosecond = Duration::from_nanos(1);
    for elapsed in [
        Duration::ZERO,
        TIMEOUT.checked_sub(nanosecond).unwrap(),
        TIMEOUT,
        TIMEOUT + nanosecond,
    ] {
        let progress = Progress {
            elapsed,
            ..mid_run()
        };
        for decide in [StopPolicy::before_call, StopPolicy::after_tools] {
            assert_eq!(
                decide(&uncapped(), &progress).is_none(),
                uncapped().time_left(elapsed).is_some(),
                "{elapsed:?}"
            );
        }
    }
}

// The token budget

#[test]
fn the_token_budget_stops_the_run_on_the_token_that_reaches_it() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..limits()
    };
    let with = |total| Progress {
        usage: tokens(total),
        ..mid_run()
    };

    for decide in [StopPolicy::before_call, StopPolicy::after_tools] {
        assert_eq!(decide(&policy, &with(999)), None);
        assert_eq!(
            decide(&policy, &with(1_000)),
            Some(StopReason::MaxTotalTokens)
        );
        assert_eq!(
            decide(&policy, &with(1_001)),
            Some(StopReason::MaxTotalTokens)
        );
    }
}

#[test]
fn the_token_budget_counts_input_plus_output_and_not_the_cache_fields_again() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..limits()
    };
    let cached = Progress {
        usage: Usage::from_inclusive(TokenCounts {
            input: 700,
            output: 200,
            reasoning: Some(0),
            cache_read: Some(600),
            cache_write: Some(100),
        }),
        ..mid_run()
    };

    assert_eq!(cached.usage.total(), 900);
    assert_eq!(policy.before_call(&cached), None);
}

#[test]
fn the_token_budget_counts_tokens_whose_parts_the_provider_did_not_report() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..limits()
    };
    let with = |input| Progress {
        usage: Usage::from_inclusive(TokenCounts {
            input,
            output: 200,
            ..TokenCounts::default()
        }),
        ..mid_run()
    };

    assert_eq!(with(800).usage.cache_read_tokens, None);
    assert_eq!(policy.before_call(&with(799)), None);
    assert_eq!(
        policy.before_call(&with(800)),
        Some(StopReason::MaxTotalTokens)
    );
    assert_eq!(
        policy.after_tools(&with(800)),
        Some(StopReason::MaxTotalTokens)
    );
}

#[test]
fn a_run_without_a_token_budget_is_never_stopped_for_tokens() {
    let progress = Progress {
        usage: Usage::from_inclusive(TokenCounts {
            input: u64::MAX,
            output: u64::MAX,
            reasoning: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        }),
        ..mid_run()
    };

    assert_eq!(limits().max_total_tokens, None);
    assert_eq!(limits().before_call(&progress), None);
    assert_eq!(limits().after_tools(&progress), None);
}

// The cap on invalid turns in a row

/// Progress after `consecutive_invalid_turns` invalid turns in a row.
fn after_invalid(consecutive_invalid_turns: u32) -> Progress {
    Progress {
        consecutive_invalid_turns,
        ..mid_run()
    }
}

#[test]
fn the_invalid_turn_cap_stops_the_run_on_the_turn_that_reaches_it() {
    assert_eq!(limits().max_consecutive_invalid_turns, cap(3));
    assert_eq!(limits().after_tools(&after_invalid(2)), None);
    assert_eq!(
        limits().after_tools(&after_invalid(3)),
        Some(StopReason::InvalidCallsExhausted)
    );
    assert_eq!(
        limits().after_tools(&after_invalid(4)),
        Some(StopReason::InvalidCallsExhausted)
    );
}

#[test]
fn the_invalid_turn_cap_is_not_read_before_a_provider_call() {
    assert_eq!(limits().before_call(&after_invalid(3)), None);
}

#[test]
fn the_smallest_invalid_turn_cap_stops_the_run_on_the_first_invalid_turn() {
    let policy = StopPolicy {
        max_consecutive_invalid_turns: cap(1),
        ..limits()
    };

    assert_eq!(policy.after_tools(&after_invalid(0)), None);
    assert_eq!(
        policy.after_tools(&after_invalid(1)),
        Some(StopReason::InvalidCallsExhausted)
    );
}

#[test]
fn a_run_without_an_invalid_turn_cap_is_never_stopped_for_invalid_turns() {
    let policy = StopPolicy {
        max_consecutive_invalid_turns: None,
        ..limits()
    };

    for invalid in [1, 3, u32::MAX] {
        assert_eq!(
            policy.after_tools(&after_invalid(invalid)),
            None,
            "{invalid}"
        );
    }
}

#[test]
fn a_run_with_no_cap_and_no_budget_is_stopped_by_its_timeout_alone() {
    let far = Progress {
        turns: u32::MAX,
        elapsed: TIMEOUT.checked_sub(Duration::from_nanos(1)).unwrap(),
        usage: tokens(u64::MAX),
        consecutive_invalid_turns: u32::MAX,
    };
    let timed_out = Progress {
        elapsed: TIMEOUT,
        ..far
    };

    for decide in [StopPolicy::before_call, StopPolicy::after_tools] {
        assert_eq!(decide(&uncapped(), &far), None);
        assert_eq!(decide(&uncapped(), &timed_out), Some(StopReason::Timeout));
    }
}

// The response, in natural mode

#[test]
fn natural_mode_completes_on_a_response_with_no_tool_calls() {
    for finish in [FinishReason::EndTurn, FinishReason::ToolUse] {
        assert_eq!(
            final_in(CompletionMode::Natural, &finish),
            StopReason::Completed,
            "{finish}"
        );
    }
}

#[test]
fn natural_mode_completes_on_a_finish_reason_it_does_not_know() {
    let unknown = FinishReason::from("eos".to_owned());

    assert_eq!(
        final_in(CompletionMode::Natural, &unknown),
        StopReason::Completed
    );
    assert_eq!(natural(&unknown, Calls::Tools), None);
    assert_eq!(
        final_in(CompletionMode::Explicit, &unknown),
        StopReason::EndedWithoutCompletion
    );
}

#[test]
fn natural_mode_goes_on_when_the_response_has_tool_calls() {
    for finish in [FinishReason::ToolUse, FinishReason::EndTurn] {
        assert_eq!(natural(&finish, Calls::Tools), None);
    }
}

#[test]
fn natural_mode_reads_task_complete_as_any_other_tool() {
    assert_eq!(natural(&FinishReason::ToolUse, Calls::TaskComplete), None);
}

// The response, in explicit mode

#[test]
fn explicit_mode_completes_when_task_complete_is_called() {
    for finish in [FinishReason::ToolUse, FinishReason::EndTurn] {
        assert_eq!(
            explicit(&finish, Calls::TaskComplete),
            Some(StopReason::Completed)
        );
    }
}

#[test]
fn explicit_mode_ends_without_completion_on_a_response_with_no_tool_calls() {
    assert_eq!(
        final_in(CompletionMode::Explicit, &FinishReason::EndTurn),
        StopReason::EndedWithoutCompletion
    );
}

#[test]
fn explicit_mode_goes_on_when_the_response_calls_other_tools() {
    assert_eq!(explicit(&FinishReason::ToolUse, Calls::Tools), None);
}

// A response the model didn't finish

const EVERY_CALLS: [Calls; 2] = [Calls::Tools, Calls::TaskComplete];

#[test]
fn a_truncated_response_is_output_truncated_whatever_it_called_so_its_tools_never_run() {
    for mode in EVERY_MODE {
        for calls in EVERY_CALLS {
            assert_eq!(
                limits().after_response(&FinishReason::MaxTokens, mode, calls),
                Some(StopReason::OutputTruncated),
                "{calls:?}"
            );
        }
        assert_eq!(
            final_in(mode, &FinishReason::MaxTokens),
            StopReason::OutputTruncated,
            "{mode} with no calls"
        );
    }
}

#[test]
fn a_response_cut_short_at_the_context_window_is_context_exhausted_whatever_it_called() {
    for mode in EVERY_MODE {
        for calls in EVERY_CALLS {
            assert_eq!(
                limits().after_response(&FinishReason::ContextWindow, mode, calls),
                Some(StopReason::ContextExhausted),
                "{calls:?}"
            );
        }
        assert_eq!(
            final_in(mode, &FinishReason::ContextWindow),
            StopReason::ContextExhausted,
            "{mode} with no calls"
        );
    }
}

#[test]
fn a_refusal_is_refused_and_never_completed_whatever_it_called() {
    for mode in EVERY_MODE {
        for calls in EVERY_CALLS {
            assert_eq!(
                limits().after_response(&FinishReason::Refusal, mode, calls),
                Some(StopReason::Refused),
                "{calls:?}"
            );
        }
        assert_eq!(
            final_in(mode, &FinishReason::Refusal),
            StopReason::Refused,
            "{mode} with no calls"
        );
    }
}

// Precedence

#[test]
fn a_response_is_judged_without_the_limits_so_one_that_finishes_completes_the_run() {
    // `after_response` takes no progress: a policy whose every limit is
    // already reached still answers from the response alone.
    let policy = StopPolicy {
        max_turns: cap(1),
        timeout: Duration::ZERO,
        max_total_tokens: Some(0),
        max_consecutive_invalid_turns: cap(1),
    };

    assert_eq!(
        policy.after_response(
            &FinishReason::ToolUse,
            CompletionMode::Natural,
            Calls::Tools
        ),
        None
    );
    assert_eq!(
        policy.after_final(&FinishReason::EndTurn, CompletionMode::Natural),
        StopReason::Completed
    );
}

#[test]
fn before_a_provider_call_the_timeout_is_reported_ahead_of_the_token_budget() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..limits()
    };

    assert_eq!(
        policy.before_call(&everything_reached(&policy)),
        Some(StopReason::Timeout)
    );
}

#[test]
fn after_the_tool_phase_the_reasons_rank_invalid_turns_then_turns_then_timeout_then_tokens() {
    let policy = StopPolicy {
        max_total_tokens: Some(1_000),
        ..limits()
    };
    let all = everything_reached(&policy);
    let without_invalid_turns = Progress {
        consecutive_invalid_turns: 0,
        ..all
    };
    let without_turns = Progress {
        turns: 1,
        ..without_invalid_turns
    };
    let without_timeout = Progress {
        elapsed: Duration::ZERO,
        ..without_turns
    };

    assert_eq!(
        policy.after_tools(&all),
        Some(StopReason::InvalidCallsExhausted)
    );
    assert_eq!(
        policy.after_tools(&without_invalid_turns),
        Some(StopReason::MaxTurns)
    );
    assert_eq!(
        policy.after_tools(&without_turns),
        Some(StopReason::Timeout)
    );
    assert_eq!(
        policy.after_tools(&without_timeout),
        Some(StopReason::MaxTotalTokens)
    );
}

// Waiting out a backoff

#[test]
fn a_wait_is_allowed_only_when_it_ends_before_the_timeout() {
    let second = Duration::from_secs(1);
    let nanosecond = Duration::from_nanos(1);
    let elapsed = TIMEOUT.checked_sub(second).unwrap();

    assert!(limits().allows_wait(elapsed, second.checked_sub(nanosecond).unwrap()));
    assert!(!limits().allows_wait(elapsed, second));
    assert!(!limits().allows_wait(elapsed, second + nanosecond));
    assert!(limits().allows_wait(Duration::ZERO, Duration::ZERO));
}

#[test]
fn a_wait_too_long_to_add_is_refused_rather_than_overflowing() {
    assert!(!limits().allows_wait(Duration::from_secs(1), Duration::MAX));
    let policy = StopPolicy {
        timeout: Duration::MAX,
        ..limits()
    };
    assert!(!policy.allows_wait(Duration::from_secs(1), Duration::MAX));
}
