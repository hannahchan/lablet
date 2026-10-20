//! The context headers: each attempt's `traceparent` names its own chat
//! span, a retry's included, a no-op propagator sends none, and a baggage
//! propagator sends the context's baggage.

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{
    Extractor, Injector, TextMapCompositePropagator, TextMapPropagator,
};
use opentelemetry::trace::FutureExt as _;
use opentelemetry::trace::noop::NoopTextMapPropagator;
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use wiremock::MockServer;

use crate::harness::{self, Recorder};

#[tokio::test]
async fn each_attempt_s_traceparent_names_its_own_chat_span_a_retry_included() {
    let server = MockServer::start().await;
    harness::answer_in_turn(
        &server,
        vec![
            harness::envelope(529, "overloaded_error", "Overloaded"),
            harness::ok(harness::text("Hello")),
        ],
    )
    .await;
    let provider = harness::with(
        harness::target(&server, false),
        TraceContextPropagator::new(),
        false,
    );
    let (recorder, tracer) = Recorder::new();

    harness::run(harness::looped(provider, Vec::new(), 1).tracer(tracer)).await;

    let received = harness::received(&server).await;
    let chats = recorder.chats();
    assert_eq!(received.len(), 2);
    assert_eq!(chats.len(), 2);
    for (request, chat) in received.iter().zip(&chats) {
        let context = &chat.span_context;
        assert_eq!(
            harness::header(request, "traceparent"),
            Some(format!("00-{}-{}-01", context.trace_id(), context.span_id()).as_str())
        );
        assert_eq!(harness::header(request, "tracestate"), None);
    }
    assert_ne!(
        harness::header(&received[0], "traceparent"),
        harness::header(&received[1], "traceparent"),
        "the retry has a span of its own"
    );
}

/// A propagator that names one key a header can't be named, one whose
/// value is empty, and one that's fine.
#[derive(Debug)]
struct Awkward(Vec<String>);

impl TextMapPropagator for Awkward {
    fn inject_context(&self, _: &Context, injector: &mut dyn Injector) {
        injector.set("bad key", "value".to_owned());
        injector.set("x-empty", String::new());
        injector.set("x-fine", "yes".to_owned());
    }

    fn extract_with_context(&self, cx: &Context, _: &dyn Extractor) -> Context {
        cx.clone()
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.0)
    }
}

#[tokio::test]
async fn a_key_or_value_a_header_can_t_carry_is_left_out_and_the_request_still_goes() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::with(
        harness::target(&server, false),
        Awkward(vec![
            "bad key".to_owned(),
            "x-empty".to_owned(),
            "x-fine".to_owned(),
        ]),
        false,
    );

    harness::complete(&provider).await.unwrap();

    let request = &harness::received(&server).await[0];
    assert_eq!(harness::header(request, "x-fine"), Some("yes"));
    assert_eq!(harness::header(request, "x-empty"), None);
    assert!(
        request
            .headers
            .iter()
            .all(|(name, _)| !name.as_str().contains(' '))
    );
}

#[tokio::test]
async fn a_no_op_propagator_sends_no_context_header() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::with(
        harness::target(&server, false),
        NoopTextMapPropagator::new(),
        false,
    );
    let (_recorder, tracer) = Recorder::new();

    harness::run(harness::looped(provider, Vec::new(), 0).tracer(tracer)).await;

    let request = &harness::received(&server).await[0];
    for name in ["traceparent", "tracestate", "baggage", "b3"] {
        assert_eq!(harness::header(request, name), None, "{name}");
    }
}

#[tokio::test]
async fn the_propagator_of_a_baggage_context_sends_baggage() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::with(
        harness::target(&server, false),
        TextMapCompositePropagator::new(vec![
            Box::new(TraceContextPropagator::new()),
            Box::new(BaggagePropagator::new()),
        ]),
        false,
    );
    let (_recorder, tracer) = Recorder::new();
    let context = Context::current_with_baggage([KeyValue::new("tenant", "acme")]);

    harness::run(harness::looped(provider, Vec::new(), 0).tracer(tracer))
        .with_context(context)
        .await;

    let request = &harness::received(&server).await[0];
    assert_eq!(harness::header(request, "baggage"), Some("tenant=acme"));
    assert!(harness::header(request, "traceparent").is_some());
}
