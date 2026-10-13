//! The inbound context, extracted through the propagators a config names
//! from environments the tests state, as a build reads them, with the
//! warnings caught by a `tracing` subscriber of the test's own.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::sync::{Arc, Mutex};

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

use super::*;

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const SPAN_ID: &str = "b7ad6b7169203331";
const SAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
const UNSAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-00";

/// The propagators the config `text` names, `None` for a config that names
/// none.
fn named(text: Option<&str>) -> Option<Propagators> {
    text.map(|text| Propagators::parse(text).unwrap())
}

/// The inbound context of an environment that holds `held` and nothing
/// else, through the propagators `propagators` names, and each warning
/// reading it logged, one line each.
fn inbound_of(propagators: Option<&str>, held: &[(&str, &str)]) -> (Inbound, Vec<String>) {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    let written = Written::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(written.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_ansi(false)
        .finish();
    let propagators = named(propagators);
    let inbound =
        tracing::subscriber::with_default(subscriber, || inbound_in(propagators.as_ref(), &env));
    let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
    (inbound, text.lines().map(str::to_owned).collect())
}

/// What the subscriber writes, kept.
#[derive(Debug, Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Written {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn parent_of(inbound: &Inbound) -> SpanContext {
    inbound.parent.span().span_context().clone()
}

#[test]
fn with_nothing_set_a_run_starts_from_the_empty_context() {
    let (inbound, warnings) = inbound_of(None, &[]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!inbound.parent.has_active_span());
    assert_eq!(inbound.parent.baggage().len(), 0);
}

#[test]
fn traceparent_and_tracestate_from_the_environment_are_the_runs_parent() {
    let (inbound, warnings) = inbound_of(
        None,
        &[
            ("TRACEPARENT", SAMPLED),
            ("TRACESTATE", "vendor=opaque,other=1"),
        ],
    );

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        parent_of(&inbound),
        SpanContext::new(
            TraceId::from_hex(TRACE_ID).unwrap(),
            SpanId::from_hex(SPAN_ID).unwrap(),
            TraceFlags::SAMPLED,
            true,
            "vendor=opaque,other=1".parse::<TraceState>().unwrap(),
        )
    );
}

#[test]
fn an_unsampled_traceparent_is_a_parent_that_is_not_sampled() {
    let (inbound, _) = inbound_of(None, &[("TRACEPARENT", UNSAMPLED)]);

    let parent = parent_of(&inbound);
    assert!(parent.is_valid() && parent.is_remote());
    assert!(!parent.is_sampled());
}

/// `propagators: none` reads no context variable, and `OTEL_PROPAGATORS` in
/// the environment changes nothing: the config alone names the propagators.
#[test]
fn propagators_none_ignores_the_context_variables_and_otel_propagators_changes_nothing() {
    let (inbound, warnings) = inbound_of(
        Some("none"),
        &[("TRACEPARENT", SAMPLED), ("BAGGAGE", "user=1")],
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!inbound.parent.has_active_span());
    assert_eq!(inbound.parent.baggage().len(), 0);

    let (inbound, _) = inbound_of(
        None,
        &[("OTEL_PROPAGATORS", "none"), ("TRACEPARENT", SAMPLED)],
    );
    assert!(
        parent_of(&inbound).is_valid(),
        "the variable isn't read, so the default propagators extract the parent"
    );
    let (inbound, _) = inbound_of(
        None,
        &[
            ("OTEL_PROPAGATORS", "b3"),
            ("B3", &format!("{TRACE_ID}-{SPAN_ID}-1")),
        ],
    );
    assert!(
        !inbound.parent.has_active_span(),
        "the variable isn't read, so `B3` isn't either"
    );
}

#[test]
fn each_propagator_extracts_only_what_it_carries() {
    let held = [("TRACEPARENT", SAMPLED), ("BAGGAGE", "user=1")];

    let (traced, _) = inbound_of(Some("tracecontext"), &held);
    assert!(parent_of(&traced).is_valid());
    assert_eq!(traced.parent.baggage().len(), 0);

    let (carried, warnings) = inbound_of(Some("baggage"), &held);
    assert!(!carried.parent.has_active_span());
    assert_eq!(carried.parent.baggage().len(), 1);
    assert!(
        warnings.is_empty(),
        "a parent nothing reads isn't warned about: {warnings:?}"
    );
}

#[test]
fn a_traceparent_that_does_not_parse_is_ignored_with_a_warning_that_names_no_value() {
    for broken in [
        "00-0AF7651916CD43DD8448EB211C80319C-B7AD6B7169203331-01",
        "00-00000000000000000000000000000000-b7ad6b7169203331-01",
        "a-secret-of-sorts",
    ] {
        let (inbound, warnings) = inbound_of(None, &[("TRACEPARENT", broken)]);

        assert!(!inbound.parent.has_active_span(), "{broken}");
        assert_eq!(
            warnings,
            [
                "`TRACEPARENT` holds a value that isn't a W3C trace parent, so it's ignored and \
              each run starts a trace of its own"
            ],
            "{broken}"
        );
    }
}

/// A B3 parent that doesn't parse is warned about by the variables that
/// hold it, under the propagator that reads them, and a `B3` that holds a
/// sampling decision alone carries no parent and is no mistake.
#[test]
fn a_b3_parent_that_does_not_parse_is_ignored_with_a_warning_that_names_no_value() {
    let broken = "0AF7651916CD43DD8448EB211C80319C-B7AD6B7169203331-1";
    let (b3, warnings) = inbound_of(Some("b3"), &[("B3", broken)]);
    assert!(!b3.parent.has_active_span());
    assert_eq!(
        warnings,
        [
            "`B3` holds a value that isn't a B3 parent, so it's ignored and each run starts a \
             trace of its own"
        ]
    );

    for multiple in [
        [
            ("X_B3_TRACEID", "a-secret-of-sorts"),
            ("X_B3_SPANID", SPAN_ID),
        ],
        [("X_B3_TRACEID", TRACE_ID), ("X_B3_SAMPLED", "1")],
    ] {
        let (b3multi, warnings) = inbound_of(Some("b3multi"), &multiple);
        assert!(!b3multi.parent.has_active_span(), "{multiple:?}");
        assert_eq!(
            warnings,
            [
                "`X_B3_TRACEID` and `X_B3_SPANID` hold no B3 parent, so they're ignored and each \
                 run starts a trace of its own"
            ],
            "{multiple:?}"
        );
    }

    for decision in ["0", "1", "d"] {
        let (b3, warnings) = inbound_of(Some("b3"), &[("B3", decision)]);
        assert!(!b3.parent.has_active_span(), "{decision}");
        assert!(warnings.is_empty(), "{decision}: {warnings:?}");
    }
    let (b3multi, warnings) = inbound_of(Some("b3multi"), &[("X_B3_SAMPLED", "0")]);
    assert!(!b3multi.parent.has_active_span());
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Baggage is extracted, since `baggage` is a default propagator, and is
/// in the context every run's spans are opened in; nothing lablet emits
/// reads it.
#[test]
fn inbound_baggage_is_in_the_runs_context() {
    let (inbound, warnings) = inbound_of(None, &[("BAGGAGE", "user=alice,tier=gold%20plus")]);

    assert!(warnings.is_empty(), "{warnings:?}");
    let baggage = inbound.parent.baggage();
    assert_eq!(
        baggage.get("user").map(ToString::to_string).as_deref(),
        Some("alice")
    );
    assert_eq!(
        baggage.get("tier").map(ToString::to_string).as_deref(),
        Some("gold plus")
    );
}

/// A member the baggage propagator can't read is dropped and the rest are
/// kept, and the warning names the variable, never a member: the SDK's own
/// warning holds the whole value.
#[test]
fn a_baggage_member_that_does_not_parse_is_ignored_with_a_warning_that_names_no_value() {
    for broken in [
        "tenant=acme,user-email=jo%40example.com,",
        "tenant=acme,user-email",
        "tenant=acme,user-email=%FF",
    ] {
        let (inbound, warnings) = inbound_of(None, &[("BAGGAGE", broken)]);

        assert_eq!(
            inbound
                .parent
                .baggage()
                .get("tenant")
                .map(ToString::to_string)
                .as_deref(),
            Some("acme"),
            "{broken}"
        );
        assert_eq!(
            warnings,
            [
                "`BAGGAGE` holds a value the baggage propagator can't read in full, so what it \
                 can't read is ignored"
            ],
            "{broken}"
        );
    }
}

/// The input reads the variable of every key the configured propagators'
/// fields name, and no other.
#[test]
fn the_input_reads_the_variable_of_every_key_the_configured_propagators_ask_for() {
    for configured in [
        None,
        Some("b3"),
        Some("b3multi"),
        Some("tracecontext,baggage,b3,b3multi"),
    ] {
        let asked: BTreeSet<String> = propagator(named(configured).as_ref())
            .fields()
            .map(variable)
            .collect();
        let read = Arc::new(Mutex::new(BTreeSet::new()));
        let reading = Arc::clone(&read);
        let env = move |name: &str| {
            reading.lock().unwrap().insert(name.to_owned());
            None
        };

        let _ = inbound_in(named(configured).as_ref(), &env);

        assert_eq!(*read.lock().unwrap(), asked, "{configured:?}");
    }
}

/// A command inherits none of the known context variables, so a
/// propagator the command line serves whose variable isn't among them
/// would leave a command another span's.
#[test]
fn the_known_context_variables_cover_every_field_of_every_propagator_the_cli_serves() {
    let every = named(Some("tracecontext,baggage,b3,b3multi"));

    let fields: BTreeSet<String> = propagator(every.as_ref()).fields().map(variable).collect();

    let known = BTreeSet::from(lablet_env_carrier::CONTEXT_VARIABLES.map(String::from));
    assert!(fields.is_subset(&known), "{:?}", fields.difference(&known));
    assert_eq!(fields.len(), 8, "{fields:?}");
}

/// What `inbound` injects of its own parent into an empty environment.
fn injected(inbound: &Inbound) -> Vec<(String, String)> {
    let mut environment = BTreeMap::new();
    inbound.propagator.inject_context(
        &inbound.parent,
        &mut lablet_env_carrier::EnvInjector::new(&mut environment, &BTreeSet::new()),
    );
    environment
        .into_iter()
        .map(|(name, value)| (name.into_string().unwrap(), value.into_string().unwrap()))
        .collect()
}

fn pairs(held: &[(&str, &str)]) -> Vec<(String, String)> {
    held.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// `b3` is the single header and `b3multi` the multiple headers, each
/// extracting from its own variables and injecting them, and neither
/// reading the other's.
#[test]
fn b3_and_b3multi_extract_and_inject_their_own_variables() {
    let single = format!("{TRACE_ID}-{SPAN_ID}-1");
    let multiple = [
        ("X_B3_SAMPLED", "1"),
        ("X_B3_SPANID", SPAN_ID),
        ("X_B3_TRACEID", TRACE_ID),
    ];
    let parent = SpanContext::new(
        TraceId::from_hex(TRACE_ID).unwrap(),
        SpanId::from_hex(SPAN_ID).unwrap(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );

    let (b3, warnings) = inbound_of(Some("b3"), &[("B3", &single)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(parent_of(&b3), parent);
    assert_eq!(injected(&b3), pairs(&[("B3", &single)]));

    let (b3multi, warnings) = inbound_of(Some("b3multi"), &multiple);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(parent_of(&b3multi), parent);
    assert_eq!(injected(&b3multi), pairs(&multiple));

    let (b3, _) = inbound_of(Some("b3"), &multiple);
    assert!(!b3.parent.has_active_span(), "b3 reads no X_B3_* variable");
    let (b3multi, _) = inbound_of(Some("b3multi"), &[("B3", &single)]);
    assert!(!b3multi.parent.has_active_span(), "b3multi reads no B3");
    let (traced, _) = inbound_of(None, &[("B3", &single), ("X_B3_TRACEID", TRACE_ID)]);
    assert!(
        !traced.parent.has_active_span(),
        "the default reads neither"
    );
}
