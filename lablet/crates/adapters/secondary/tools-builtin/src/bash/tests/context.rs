//! The environment a command starts with, built from environments and
//! contexts the tests state: what a command would inherit can't be set in
//! a test's own process.

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::noop::NoopTextMapPropagator;
use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId};
use opentelemetry::{KeyValue, trace::TraceState};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};

use super::*;

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const SPAN_ID: &str = "b7ad6b7169203331";
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// A context whose span is the sampled one [`TRACEPARENT`] names, with
/// `state` as its trace state.
fn under(state: &str) -> Context {
    Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_hex(TRACE_ID).unwrap(),
        SpanId::from_hex(SPAN_ID).unwrap(),
        TraceFlags::SAMPLED,
        true,
        state.parse::<TraceState>().unwrap(),
    ))
}

/// What a command inherits from lablet's environment: a `PATH`, and a
/// value of every context variable, each another span's.
const INHERITED: [(&str, &str); 10] = [
    ("PATH", "/usr/bin:/bin"),
    (
        "TRACEPARENT",
        "00-11111111111111111111111111111111-2222222222222222-01",
    ),
    ("TRACESTATE", "lablet=its-own"),
    ("BAGGAGE", "lablet=its-own"),
    ("B3", "11111111111111111111111111111111-2222222222222222-1"),
    ("X_B3_TRACEID", "11111111111111111111111111111111"),
    ("X_B3_SPANID", "2222222222222222"),
    ("X_B3_PARENTSPANID", "3333333333333333"),
    ("X_B3_SAMPLED", "1"),
    ("X_B3_FLAGS", "1"),
];

/// A `Bash` whose commands would start with `environment`, of which the
/// settings state `stated`, injected through `propagator`.
fn bash(
    environment: &[(&str, &str)],
    stated: &[&str],
    propagator: impl TextMapPropagator + Send + Sync + 'static,
) -> Bash {
    Bash {
        root: Root::open(&std::env::temp_dir()).unwrap(),
        environment: environment
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect(),
        kept: stated.iter().map(|&name| name.to_owned()).collect(),
        propagator: Arc::new(propagator),
    }
}

fn held(environment: &BTreeMap<OsString, OsString>) -> Vec<(&str, &str)> {
    environment
        .iter()
        .map(|(name, value)| (name.to_str().unwrap(), value.to_str().unwrap()))
        .collect()
}

/// A propagator of a vendor's own, which names `x-vendor-trace` and
/// injects nothing.
#[derive(Debug)]
struct Vendor(Vec<String>);

impl Vendor {
    fn new() -> Self {
        Self(vec!["x-vendor-trace".to_owned()])
    }
}

impl TextMapPropagator for Vendor {
    fn inject_context(&self, _: &Context, _: &mut dyn Injector) {}

    fn extract_with_context(&self, cx: &Context, _: &dyn Extractor) -> Context {
        cx.clone()
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.0)
    }
}

/// `none` injects nothing and names no variable, so the list of every
/// context variable is what clears the ones a command would inherit.
#[test]
fn an_inherited_context_variable_is_removed_when_the_propagator_injects_none() {
    let bash = bash(&INHERITED, &[], NoopTextMapPropagator::new());

    let environment = bash.environment_in(&under(""));

    assert_eq!(held(&environment), [("PATH", "/usr/bin:/bin")]);
}

/// The trace context propagator injects nothing for a context with no
/// span, which would leave the inherited `TRACEPARENT` in place.
#[test]
fn an_inherited_traceparent_is_removed_when_there_is_no_span_to_inject() {
    let bash = bash(&INHERITED, &[], TraceContextPropagator::new());

    let environment = bash.environment_in(&Context::new());

    assert_eq!(held(&environment), [("PATH", "/usr/bin:/bin")]);
}

#[test]
fn a_variable_the_propagator_names_is_never_inherited() {
    let bash = bash(
        &[
            ("PATH", "/usr/bin:/bin"),
            ("X_VENDOR_TRACE", "another span's"),
        ],
        &[],
        Vendor::new(),
    );

    let environment = bash.environment_in(&under(""));

    assert_eq!(held(&environment), [("PATH", "/usr/bin:/bin")]);
}

/// The config wins key by key: the `TRACEPARENT` it states stays, and the
/// `TRACESTATE` it doesn't is the injected one.
#[test]
fn a_context_variable_the_config_states_wins_over_the_injected_one() {
    let stated = "00-33333333333333333333333333333333-4444444444444444-01";
    let bash = bash(
        &[("PATH", "/usr/bin:/bin"), ("TRACEPARENT", stated)],
        &["TRACEPARENT", "B3"],
        TraceContextPropagator::new(),
    );

    let environment = bash.environment_in(&under("vendor=1"));

    assert_eq!(
        held(&environment),
        [
            ("PATH", "/usr/bin:/bin"),
            ("TRACEPARENT", stated),
            ("TRACESTATE", "vendor=1"),
        ]
    );
}

#[test]
fn a_command_s_context_is_injected_in_place_of_the_inherited_one_and_an_empty_trace_state_is_not() {
    let bash = bash(
        &INHERITED,
        &[],
        opentelemetry::propagation::TextMapCompositePropagator::new(vec![
            Box::new(TraceContextPropagator::new()),
            Box::new(BaggagePropagator::new()),
        ]),
    );

    let context = under("").with_baggage([KeyValue::new("tenant", "acme")]);
    let environment = bash.environment_in(&context);

    assert_eq!(
        held(&environment),
        [
            ("BAGGAGE", "tenant=acme"),
            ("PATH", "/usr/bin:/bin"),
            ("TRACEPARENT", TRACEPARENT),
        ]
    );
}
