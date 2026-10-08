//! O23: a host that runs lablet as a library provides its OpenTelemetry.
//! Two `Lablet`s in one process, each given providers of its own, each
//! reach only their own, the host's own `tracing` subscriber sees lablet's
//! diagnostics and none of its telemetry, and a run under a span the host
//! has open is that span's child, in the host's trace.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use opentelemetry::Context as OtelContext;
use opentelemetry::trace::{
    FutureExt as _, Span as _, TraceContextExt as _, Tracer as _, TracerProvider as _,
};
use serde_json::json;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use crate::harness::{ENDS, Lab, Traced, request};
use crate::key;

/// One event as a host's subscriber was told it: its target and the names
/// of its fields.
type Event = (String, BTreeSet<String>);

/// What a host's subscriber was told, in order.
#[derive(Default)]
struct Heard(Arc<Mutex<Vec<Event>>>);

struct Names(BTreeSet<String>);

impl Visit for Names {
    fn record_debug(&mut self, field: &Field, _: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned());
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Heard {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut names = Names(BTreeSet::new());
        event.record(&mut names);
        self.0
            .lock()
            .unwrap()
            .push((event.metadata().target().to_owned(), names.0));
    }
}

/// Whether `text` names a registry attribute or signal: lablet's telemetry
/// is under these namespaces, and its diagnostics under neither.
fn names_telemetry(text: &str) -> bool {
    text.starts_with("gen_ai.") || text.starts_with("lablet.")
}

/// The subscriber is the test thread's, which is where the loop runs and
/// where the run's own diagnostics are written; what the export threads and
/// the blocking tasks write goes to the global default, which a shared test
/// binary can't set. Lablet's telemetry passes through `tracing` on no
/// thread, so the thread this hears is the one that would have heard it.
#[tokio::test]
async fn two_lablets_given_their_own_providers_each_reach_only_their_own() {
    let heard = Heard::default();
    let told = Arc::clone(&heard.0);
    let subscriber = tracing_subscriber::registry().with(heard);
    let _host = tracing::subscriber::set_default(subscriber);

    let first = Lab::new("hosts-first");
    let second = Lab::new("hosts-second");
    let mut lablets = Vec::new();
    // The first `Lablet`'s transcripts can't be written, with a file where
    // their directory would be, so its runs warn on the diagnostic log,
    // which the host's subscriber is to hear.
    let in_the_way = first.write("in-the-way", "");
    for (lab, more) in [
        (
            &first,
            json!({
                "prompt": { "system": "You answer tersely." },
                "run": { "transcript_path": in_the_way.join("run.json") },
            }),
        ),
        (
            &second,
            json!({ "prompt": { "system": "You answer at length." } }),
        ),
    ] {
        let config = lab.config(ENDS, more);
        lablets.push(lab.build(config).await.unwrap());
    }
    let mut runs: Vec<Vec<String>> = Vec::new();
    for lablet in &mut lablets {
        let mut ids = Vec::new();
        for _ in 0..2 {
            let finished = lablet.run(request()).await;
            ids.push(finished.summary.outcome.run_id.to_string());
        }
        runs.push(ids);
    }
    for lablet in lablets {
        lablet.shutdown().await;
    }

    // Each host's providers hold their own `Lablet`'s two runs' spans and
    // records, and nothing of the other's.
    for (lab, own) in [(&first, &runs[0]), (&second, &runs[1])] {
        let exported = lab.exported();
        assert_eq!(exported.spans_of(key::INVOKE_AGENT).len(), 2);
        assert_eq!(exported.records_of(key::WIDE_EVENT).len(), 2);
        let of_runs: BTreeSet<&str> = exported
            .spans
            .iter()
            .map(|span| &span.attributes)
            .chain(exported.records.iter().map(|record| &record.attributes))
            .map(|attributes| attributes[key::GEN_AI_CONVERSATION_ID].as_str().unwrap())
            .collect();
        assert_eq!(
            of_runs,
            own.iter().map(String::as_str).collect::<BTreeSet<_>>(),
            "{}",
            lab.path().display()
        );
        for run_id in own {
            let traced = Traced::of(&exported, run_id);
            assert_eq!(traced.spans.len(), 2, "a root and one attempt");
            assert_eq!(traced.wide().trace_id, traced.root().trace_id);
        }
    }

    // The host's subscriber heard diagnostics at most, never a record of the
    // telemetry: no event whose target or field names a registry attribute
    // or signal.
    let heard = told.lock().unwrap().clone();
    let warned: Vec<_> = heard
        .iter()
        .filter(|(target, fields)| {
            target.starts_with("lablet") && fields.contains("run_id") && fields.contains("message")
        })
        .collect();
    assert_eq!(
        warned.len(),
        2,
        "the host's subscriber heard each unwritten transcript: {heard:?}"
    );
    let telemetry: Vec<_> = heard
        .iter()
        .filter(|(target, fields)| {
            names_telemetry(target) || fields.iter().any(|field| names_telemetry(field))
        })
        .collect();
    assert!(
        telemetry.is_empty(),
        "the host's subscriber was handed telemetry: {telemetry:?}"
    );
}

/// The spans' flags on the wire: sampled, with the bit that says whether
/// the parent is remote set to known, and the parent not remote.
const LOCAL_PARENT: u32 = 0x101;

#[tokio::test]
async fn a_run_under_a_span_the_host_has_open_is_its_child_and_reaches_the_host_s_tracer() {
    let scratch = Lab::new("hosts-span");
    let mut lablet = scratch
        .build(scratch.config(ENDS, json!({})))
        .await
        .unwrap();
    // The host's own span, opened through the tracer provider it handed in.
    let operation = scratch
        .host()
        .tracer_provider()
        .tracer("host")
        .start("host operation");
    let host = operation.span_context().clone();
    let within = OtelContext::current_with_span(operation);

    let finished = lablet.run(request()).with_context(within.clone()).await;
    within.span().end();
    lablet.shutdown().await;

    let exported = scratch.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    let root = traced.root();
    assert_eq!(
        root.parent_span_id.as_deref(),
        Some(host.span_id().to_string().as_str()),
        "the root span is the host's span's child"
    );
    assert_eq!(
        root.trace_id,
        host.trace_id().to_string(),
        "the run is in the host's trace"
    );
    assert_eq!(traced.spans.len(), 2, "a root and one attempt");
    for span in &traced.spans {
        assert_eq!(span.trace_id, root.trace_id, "{}", span.name);
        assert_eq!(span.flags, LOCAL_PARENT, "{}", span.name);
    }
    assert_eq!(
        traced.chats()[0].parent_span_id.as_deref(),
        Some(root.span_id.as_str()),
        "the attempt is under the root"
    );
    assert_eq!(traced.wide().trace_id, root.trace_id);

    // The host's tracer provider holds the host's span beside the run's.
    let hosts: Vec<_> = exported
        .spans
        .iter()
        .filter(|span| span.name == "host operation")
        .collect();
    assert_eq!(hosts.len(), 1, "{:?}", exported.spans);
    assert_eq!(hosts[0].span_id, host.span_id().to_string());
}
