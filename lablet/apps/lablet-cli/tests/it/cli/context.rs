//! The inbound context and the SDK's own environment through the binary,
//! with the variables given to the process, since a test can't set one of
//! its own: a run under an inbound `TRACEPARENT` is its child, a run under
//! one that isn't sampled exports no span, and what the SDK says of a
//! setting it read for itself stays out of the diagnostic log.

use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};
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

/// Runs `script` in a lab of its own, with its telemetry in its file and
/// no network exporter, and `env` given to the process.
fn completed(test: &str, script: &str, env: &[(&str, &str)]) -> (Lab, Ran) {
    let lab = Lab::new(test);
    lab.write("work/notes.md", "Nothing yet.");
    lab.write_config(
        script,
        json!({
            "tools": lab.builtin_tools(&["read_file"]),
            "telemetry": { "otlp": { "enabled": false }, "capture_content": true },
        }),
    );
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
    command.envs(env.iter().copied());
    let run = ran(command, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], "completed", "{run:?}");
    (lab, run)
}

#[test]
fn a_run_under_an_inbound_traceparent_is_a_child_of_it_and_carries_its_tracestate() {
    let (lab, _) = completed(
        "context-child",
        READS,
        &[("TRACEPARENT", SAMPLED), ("TRACESTATE", TRACESTATE)],
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

#[test]
fn with_otel_propagators_none_a_run_is_a_trace_of_its_own() {
    let (lab, _) = completed(
        "context-none",
        ENDS,
        &[("OTEL_PROPAGATORS", "none"), ("TRACEPARENT", SAMPLED)],
    );

    let exported = lab.exported();
    let root = exported.spans_of(key::INVOKE_AGENT)[0];
    assert_eq!(root.parent_span_id, None);
    assert_ne!(root.trace_id, TRACE_ID);
    assert_eq!(root.trace_state, "");
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

/// The SDK's tracer provider reads `OTEL_TRACES_SAMPLER` for itself, in
/// lower case only, and warns of the fallback it would use; lablet reads it
/// in any case and states the sampler, so that warning would name one the
/// run doesn't use. A warning of lablet's own still shows the log is on.
#[test]
fn the_sdks_warning_about_a_sampler_it_read_itself_stays_out_of_the_log() {
    for (sampler, spans) in [("ALWAYS_OFF", 0), ("traceidratio", 2)] {
        let (lab, run) = completed(
            &format!("context-sampler-{}", sampler.to_ascii_lowercase()),
            ENDS,
            &[("OTEL_TRACES_SAMPLER", sampler), ("OTEL_PROPAGATORS", "b3")],
        );

        assert!(
            run.stderr
                .contains("`OTEL_PROPAGATORS` holds `b3`, which isn't one lablet serves"),
            "{run:?}"
        );
        for said in ["TracerProvider.Config", "fallback", "OTEL_TRACES_SAMPLER"] {
            assert!(!run.stderr.contains(said), "{sampler}: {said}: {run:?}");
        }
        assert_eq!(lab.exported().spans.len(), spans, "{sampler}");
    }
}
