//! The inbound context through the binary, with the variables given to the
//! process, since a test can't set one of its own: a run under an inbound
//! `TRACEPARENT` is its child, a run under one that isn't sampled exports no
//! span, a command is under its tool span, a run such a command starts is a
//! child of that span, `telemetry.propagators` names the propagators where
//! `OTEL_PROPAGATORS` changes nothing, and what the SDK says of a sampler
//! variable it read for itself stays out of the diagnostic log.

use lablet_test_support::conformance::otlp::Exported;
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};
use crate::harness::json_of;
use crate::key;

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT_ID: &str = "b7ad6b7169203331";
const SAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
const UNSAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-00";
const TRACESTATE: &str = "vendor=opaque,other=1";

/// The bit of a span's OTLP flags that says whether its parent is known to
/// be remote or not, and the bit that says it is.
const HAS_IS_REMOTE: u32 = 0x100;
const IS_REMOTE: u32 = 0x200;

/// A response that calls `read_file`, then one that ends the run, so a run
/// has a root, two provider calls and a tool call.
const READS: &str = "
- response:
    content:
      - tool_use: { id: call_1, name: read_file, input: { json: { path: notes.md } } }
    finish: tool_use
- response:
    content:
      - text: The notes are read.
    finish: end_turn
";

/// A response that calls `bash` with a command that prints every context
/// variable it was given, then one that ends the run, so a run has a root,
/// two provider calls and a tool call.
const PRINTS: &str = "
- response:
    content:
      - tool_use:
          id: call_1
          name: bash
          input: { json: { command: \"env | grep -E '^(TRACEPARENT|TRACESTATE|BAGGAGE|B3|X_B3_[A-Z]+)=' | sort\" } }
    finish: tool_use
- response:
    content:
      - text: The context is printed.
    finish: end_turn
";

/// Runs `script` in a lab of its own, with its telemetry in its file and
/// no network exporter, and `env` given to the process.
fn completed(test: &str, script: &str, env: &[(&str, &str)]) -> (Lab, Ran) {
    completed_with(test, script, json!({}), &Value::Null, env)
}

/// [`completed`], with `telemetry.propagators` stating `propagators`.
fn completed_through(
    test: &str,
    script: &str,
    propagators: &str,
    env: &[(&str, &str)],
) -> (Lab, Ran) {
    completed_with(test, script, json!({}), &json!(propagators), env)
}

/// [`completed`], with `tools.builtin.env` stating `stated` and
/// `telemetry.propagators` stating `propagators`.
fn completed_with(
    test: &str,
    script: &str,
    stated: Value,
    propagators: &Value,
    env: &[(&str, &str)],
) -> (Lab, Ran) {
    let lab = Lab::new(test);
    lab.write("work/notes.md", "Nothing yet.");
    let mut builtin = lab.builtin(&["bash", "read_file"]);
    builtin["env"] = stated;
    lab.write_config(
        script,
        json!({
            "tools": { "builtin": builtin },
            "telemetry": { "capture_content": true, "propagators": propagators },
            "run": { "transcript_path": "transcript.json" },
        }),
    );
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
    command.envs(env.iter().copied());
    let run = ran(command, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], "completed", "{run:?}");
    (lab, run)
}

/// What the run's one command printed, each context variable it was
/// given, as `NAME=value` lines, and its exit code.
fn printed(lab: &Lab) -> String {
    let transcript = json_of(&lab.at("transcript.json"));
    transcript["turns"][0]["tool_calls"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["text"].as_str().unwrap())
        .collect()
}

/// The id of the run's one tool span.
fn tool_span(lab: &Lab) -> String {
    let exported = lab.exported();
    let tools = exported.spans_of("execute_tool");
    assert_eq!(tools.len(), 1, "{:?}", exported.spans);
    tools[0].span_id.clone()
}

#[test]
fn a_run_under_an_inbound_traceparent_is_a_child_of_it_and_carries_its_tracestate() {
    let (lab, _) = completed(
        "context-child",
        PRINTS,
        &[("TRACEPARENT", SAMPLED), ("TRACESTATE", TRACESTATE)],
    );

    assert_eq!(
        printed(&lab),
        format!(
            "TRACEPARENT=00-{TRACE_ID}-{}-01\nTRACESTATE={TRACESTATE}\nexit code: 0",
            tool_span(&lab)
        ),
        "a command is under its tool span, in the inbound trace"
    );

    let exported = lab.exported();
    assert_eq!(exported.spans.len(), 4, "{:?}", exported.spans);
    let roots = exported.spans_of(key::INVOKE_AGENT);
    assert_eq!(roots.len(), 1);
    let root = roots[0];
    assert_eq!(root.parent_span_id.as_deref(), Some(PARENT_ID));
    assert_eq!(
        root.flags & (HAS_IS_REMOTE | IS_REMOTE),
        HAS_IS_REMOTE | IS_REMOTE,
        "the root's parent is remote: {:#x}",
        root.flags
    );
    for span in &exported.spans {
        assert_eq!(span.trace_id, TRACE_ID, "{}", span.name);
        assert_eq!(span.trace_state, TRACESTATE, "{}", span.name);
        assert_eq!(span.flags & 0xff, 1, "{} is sampled", span.name);
        if span.span_id != root.span_id {
            assert_eq!(
                span.parent_span_id.as_ref(),
                Some(&root.span_id),
                "{}",
                span.name
            );
            assert_eq!(
                span.flags & (HAS_IS_REMOTE | IS_REMOTE),
                HAS_IS_REMOTE,
                "{}'s parent is local",
                span.name
            );
        }
    }
    let wide = exported.records_of(key::WIDE_EVENT);
    assert_eq!(wide.len(), 1);
    assert_eq!(wide[0].trace_id, TRACE_ID);
    assert_eq!(wide[0].span_id, root.span_id);
}

/// The default sampler is parent-based, so it follows the inbound parent:
/// no span of the run is recorded, and its records are still exported, in
/// the inbound trace.
#[test]
fn a_run_under_an_unsampled_traceparent_exports_its_records_and_no_span() {
    let (lab, _) = completed("context-unsampled", READS, &[("TRACEPARENT", UNSAMPLED)]);

    let exported = lab.exported();
    assert!(exported.spans.is_empty(), "{:?}", exported.spans);
    assert_eq!(exported.records_of(key::WIDE_EVENT).len(), 1);
    assert!(exported.records.len() > 1, "content records beside it");
    for record in &exported.records {
        assert_eq!(record.trace_id, TRACE_ID, "{}", record.event_name);
        assert_eq!(record.flags, 0, "{} isn't sampled", record.event_name);
    }
}

/// `propagators: none` reads no context variable, so a run is a trace of
/// its own; `OTEL_PROPAGATORS=none` in the environment isn't read, so under
/// it a run is still its parent's child.
#[test]
fn with_propagators_none_a_run_is_a_trace_of_its_own_and_otel_propagators_changes_nothing() {
    let (lab, _) = completed_through(
        "context-none",
        ENDS,
        "none",
        &[
            ("OTEL_PROPAGATORS", "tracecontext"),
            ("TRACEPARENT", SAMPLED),
        ],
    );

    let exported = lab.exported();
    let root = exported.spans_of(key::INVOKE_AGENT)[0];
    assert_eq!(root.parent_span_id, None);
    assert_ne!(root.trace_id, TRACE_ID);
    assert_eq!(root.trace_state, "");

    let (lab, _) = completed(
        "context-variable-none",
        ENDS,
        &[("OTEL_PROPAGATORS", "none"), ("TRACEPARENT", SAMPLED)],
    );
    let root = lab.exported().spans_of(key::INVOKE_AGENT)[0].clone();
    assert_eq!(root.parent_span_id.as_deref(), Some(PARENT_ID));
}

#[test]
fn a_traceparent_that_does_not_parse_is_warned_of_by_name_and_the_run_is_a_trace_of_its_own() {
    let broken = "00-0AF7651916CD43DD8448EB211C80319C-B7AD6B7169203331-01";
    let (lab, run) = completed("context-broken", ENDS, &[("TRACEPARENT", broken)]);

    assert!(
        run.stderr
            .contains("`TRACEPARENT` holds a value that isn't a W3C trace parent, so it's ignored"),
        "{run:?}"
    );
    assert!(
        !run.stderr.to_ascii_lowercase().contains(&TRACE_ID[..12]),
        "{run:?}"
    );
    let root = lab.exported().spans_of(key::INVOKE_AGENT)[0].clone();
    assert_eq!(root.parent_span_id, None);
}

/// `baggage` is a default propagator, so an inbound `BAGGAGE` is extracted
/// into every run's context; nothing lablet emits carries it, the content
/// it captures and the diagnostic log included. The trailing comma is a
/// member the propagator can't read, whose warning from the SDK holds the
/// whole value.
#[test]
fn inbound_baggage_is_emitted_nowhere() {
    let (lab, run) = completed(
        "context-baggage",
        READS,
        &[
            ("TRACEPARENT", SAMPLED),
            ("BAGGAGE", "carried=baggage-0123456789,"),
        ],
    );

    assert!(
        run.stderr
            .contains("`BAGGAGE` holds a value the baggage propagator can't read in full"),
        "{run:?}"
    );
    let text = std::fs::read_to_string(lab.telemetry()).unwrap();
    assert!(!text.is_empty());
    for held in ["baggage-0123456789", "carried"] {
        assert!(!text.contains(held), "{held}");
        assert!(!run.stderr.contains(held), "{held}: {run:?}");
        assert!(!run.stdout.contains(held), "{held}: {run:?}");
    }
}

/// The SDK's tracer provider reads `OTEL_TRACES_SAMPLER` for itself and
/// warns of the fallback it would use; lablet reads no sampler variable and
/// states the sampler, parent-based always on, so the variable decides
/// nothing and that warning would name a sampler the run doesn't use. A
/// warning of lablet's own, about a `TRACEPARENT` it can't read, still
/// shows the log is on.
#[test]
fn the_sdks_warning_about_a_sampler_it_read_itself_stays_out_of_the_log() {
    for sampler in ["ALWAYS_OFF", "always_off", "traceidratio"] {
        let (lab, run) = completed(
            &format!("context-sampler-{}", sampler.to_ascii_lowercase()),
            ENDS,
            &[
                ("OTEL_TRACES_SAMPLER", sampler),
                ("OTEL_TRACES_SAMPLER_ARG", "0"),
                ("TRACEPARENT", "not-a-trace-parent"),
            ],
        );

        assert!(
            run.stderr
                .contains("`TRACEPARENT` holds a value that isn't a W3C trace parent"),
            "{run:?}"
        );
        for said in ["TracerProvider.Config", "fallback", "OTEL_TRACES_SAMPLER"] {
            assert!(!run.stderr.contains(said), "{sampler}: {said}: {run:?}");
        }
        assert_eq!(
            lab.exported().spans.len(),
            2,
            "{sampler}: the variable decides nothing"
        );
    }
}

/// `propagators: b3` and `b3multi` each extract the run's parent from their
/// own variables and inject a command's from its tool span into them, and a
/// command inherits none of the others. `OTEL_PROPAGATORS=b3` in the
/// environment changes nothing: under the default propagators `B3` isn't
/// read and `TRACEPARENT` is.
#[test]
fn b3_and_b3multi_carry_the_inbound_context_to_a_command_under_its_tool_span() {
    let single = format!("{TRACE_ID}-{PARENT_ID}-1");
    let other = "00-11111111111111111111111111111111-2222222222222222-01";
    for (propagators, inbound) in [
        ("b3", vec![("B3", single.as_str())]),
        (
            "b3multi",
            vec![
                ("X_B3_TRACEID", TRACE_ID),
                ("X_B3_SPANID", PARENT_ID),
                ("X_B3_SAMPLED", "1"),
            ],
        ),
    ] {
        let mut env = vec![
            ("OTEL_PROPAGATORS", "tracecontext"),
            ("TRACEPARENT", other),
            ("BAGGAGE", "inherited=1"),
        ];
        env.extend(inbound);
        let (lab, _) =
            completed_through(&format!("context-{propagators}"), PRINTS, propagators, &env);

        let root = lab.exported().spans_of(key::INVOKE_AGENT)[0].clone();
        assert_eq!(root.trace_id, TRACE_ID, "{propagators}");
        assert_eq!(
            root.parent_span_id.as_deref(),
            Some(PARENT_ID),
            "{propagators}"
        );
        let span = tool_span(&lab);
        let expected = match propagators {
            "b3" => format!("B3={TRACE_ID}-{span}-1"),
            _ => format!("X_B3_SAMPLED=1\nX_B3_SPANID={span}\nX_B3_TRACEID={TRACE_ID}"),
        };
        assert_eq!(
            printed(&lab),
            format!("{expected}\nexit code: 0"),
            "{propagators}"
        );
    }

    let (lab, _) = completed(
        "context-b3-variable",
        ENDS,
        &[
            ("OTEL_PROPAGATORS", "b3"),
            ("B3", &single),
            ("TRACEPARENT", other),
        ],
    );
    let root = lab.exported().spans_of(key::INVOKE_AGENT)[0].clone();
    assert_eq!(
        root.trace_id, "11111111111111111111111111111111",
        "`OTEL_PROPAGATORS` isn't read: the default propagators extract `TRACEPARENT`"
    );
}

/// Under `propagators: none`, nothing is injected, and a context variable
/// the command would inherit is another span's, so it's removed.
#[test]
fn under_propagators_none_a_command_gets_no_context_variable() {
    let (lab, _) = completed_through(
        "context-command-none",
        PRINTS,
        "none",
        &[
            ("TRACEPARENT", SAMPLED),
            ("TRACESTATE", TRACESTATE),
            ("BAGGAGE", "inherited=1"),
            ("B3", "inherited"),
            ("X_B3_SAMPLED", "1"),
        ],
    );

    assert_eq!(printed(&lab), "exit code: 0");
}

/// The config wins over what's injected, key by key: the stated
/// `TRACEPARENT` is the command's, and the `TRACESTATE` it doesn't state is
/// the tool span's.
#[test]
fn a_traceparent_the_config_states_is_the_one_a_command_gets() {
    let stated = "00-11111111111111111111111111111111-2222222222222222-01";
    let (lab, _) = completed_with(
        "context-command-stated",
        PRINTS,
        json!({ "TRACEPARENT": stated }),
        &Value::Null,
        &[("TRACEPARENT", SAMPLED), ("TRACESTATE", TRACESTATE)],
    );

    assert_eq!(
        printed(&lab),
        format!("TRACEPARENT={stated}\nTRACESTATE={TRACESTATE}\nexit code: 0")
    );
}

/// A command that runs lablet again starts a run that's a child of the
/// command's tool span, in the trace of the run that started it, though
/// each run has its config and its telemetry file of its own.
#[test]
fn a_run_a_command_starts_is_a_child_of_that_command_s_tool_span() {
    let inner = Lab::new("context-nested-inner");
    let config = inner.write(CONFIG, &inner.tree(ENDS, json!({})).to_string());
    let command = format!(
        "'{}' run --config '{}' --prompt {PROMPT:?} > /dev/null",
        env!("CARGO_BIN_EXE_lablet"),
        config.display()
    );
    let script = format!(
        "
- response:
    content:
      - tool_use: {{ id: call_1, name: bash, input: {{ json: {{ command: {} }} }} }}
    finish: tool_use
- response:
    content:
      - text: The inner run is done.
    finish: end_turn
",
        serde_json::to_string(&command).unwrap()
    );

    let (outer, _) = completed("context-nested", &script, &[]);

    let printed = printed(&outer);
    assert!(printed.ends_with("\nexit code: 0"), "{printed}");
    let exported = outer.exported();
    let tools = exported.spans_of("execute_tool");
    assert_eq!(tools.len(), 1, "{:?}", exported.spans);
    let nested = Exported::read(&inner.telemetry()).unwrap();
    let roots = nested.spans_of(key::INVOKE_AGENT);
    assert_eq!(roots.len(), 1, "{:?}", nested.spans);
    assert_eq!(roots[0].trace_id, tools[0].trace_id);
    assert_eq!(
        roots[0].parent_span_id.as_deref(),
        Some(tools[0].span_id.as_str())
    );
}

/// An inbound `BAGGAGE` holding a given credential's value is dropped by
/// value, and isn't injected again: a command gets the tool span's
/// `TRACEPARENT` and no `BAGGAGE`, though one holding nothing of a
/// credential reaches it.
#[test]
fn a_context_variable_dropped_by_value_is_not_injected_again() {
    let token = "ghp_0123456789abcdef0123456789";
    let printed_under = |test: &str, baggage: &str| {
        let lab = Lab::new(test);
        lab.write("work/notes.md", "Nothing yet.");
        let mut builtin = lab.builtin(&["bash"]);
        builtin["credentials"] = json!({ "GH": "${LABLET_TEST_GIVEN}" });
        lab.write_config(
            PRINTS,
            json!({
                "tools": { "builtin": builtin },
                "telemetry": { "propagators": "tracecontext,baggage" },
                "run": { "transcript_path": "transcript.json" },
            }),
        );
        let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
        command.envs([
            ("LABLET_TEST_GIVEN", token),
            ("TRACEPARENT", SAMPLED),
            ("BAGGAGE", baggage),
        ]);
        let run = ran(command, "");
        assert_eq!(run.code, Some(0), "{run:?}");
        printed(&lab)
    };

    let dropped = printed_under("context-dropped-baggage", &format!("k={token}"));
    let carried = printed_under("context-carried-baggage", "k=plain-0123456789abcdef");

    assert!(!dropped.contains("BAGGAGE"), "{dropped}");
    assert!(dropped.contains("TRACEPARENT=00-"), "{dropped}");
    assert!(
        carried.contains("BAGGAGE=k=plain-0123456789abcdef"),
        "{carried}"
    );
}
