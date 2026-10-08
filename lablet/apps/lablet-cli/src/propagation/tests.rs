//! The inbound context, extracted from environments the tests state as a
//! build reads them, with the warnings caught by a `tracing` subscriber of
//! the test's own.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::sync::{Arc, Mutex};

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

use super::*;
use crate::otel_env::OtelEnv;

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const SPAN_ID: &str = "b7ad6b7169203331";
const SAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
const UNSAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-00";

/// The inbound context of an environment that holds `held` and nothing
/// else, and each warning reading it logged, one line each.
fn inbound_of(held: &[(&str, &str)]) -> (Inbound, Vec<String>) {
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
    let inbound =
        tracing::subscriber::with_default(subscriber, || inbound(&OtelEnv::read(&env).context));
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
    let (inbound, warnings) = inbound_of(&[]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!inbound.parent.has_active_span());
    assert_eq!(inbound.parent.baggage().len(), 0);
}

#[test]
fn traceparent_and_tracestate_from_the_environment_are_the_runs_parent() {
    let (inbound, warnings) = inbound_of(&[
        ("TRACEPARENT", SAMPLED),
        ("TRACESTATE", "vendor=opaque,other=1"),
    ]);

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
    let (inbound, _) = inbound_of(&[("TRACEPARENT", UNSAMPLED)]);

    let parent = parent_of(&inbound);
    assert!(parent.is_valid() && parent.is_remote());
    assert!(!parent.is_sampled());
}

#[test]
fn otel_propagators_none_ignores_them() {
    let (inbound, warnings) = inbound_of(&[
        ("OTEL_PROPAGATORS", "none"),
        ("TRACEPARENT", SAMPLED),
        ("BAGGAGE", "user=1"),
    ]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!inbound.parent.has_active_span());
    assert_eq!(inbound.parent.baggage().len(), 0);
}

#[test]
fn each_propagator_extracts_only_what_it_carries() {
    let held = [("TRACEPARENT", SAMPLED), ("BAGGAGE", "user=1")];

    let (traced, _) = inbound_of(&[&held[..], &[("OTEL_PROPAGATORS", "tracecontext")]].concat());
    assert!(parent_of(&traced).is_valid());
    assert_eq!(traced.parent.baggage().len(), 0);

    let (carried, warnings) = inbound_of(&[&held[..], &[("OTEL_PROPAGATORS", "baggage")]].concat());
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
        let (inbound, warnings) = inbound_of(&[("TRACEPARENT", broken)]);

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

/// Baggage is extracted, since `baggage` is a default propagator, and is
/// in the context every run's spans are opened in; nothing lablet emits
/// reads it.
#[test]
fn inbound_baggage_is_in_the_runs_context() {
    let (inbound, warnings) = inbound_of(&[("BAGGAGE", "user=alice,tier=gold%20plus")]);

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
        let (inbound, warnings) = inbound_of(&[("BAGGAGE", broken)]);

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

/// The seam reads the context variables the propagators' fields name,
/// whichever propagators `OTEL_PROPAGATORS` names.
#[test]
fn the_seam_reads_the_variable_of_every_key_the_propagators_ask_for() {
    for named in [
        None,
        Some("b3"),
        Some("b3multi"),
        Some("tracecontext,baggage,b3,b3multi"),
    ] {
        let propagators = |name: &str| {
            (name == "OTEL_PROPAGATORS")
                .then(|| named.map(OsString::from))
                .flatten()
        };
        let asked: BTreeSet<String> = OtelEnv::read(&propagators)
            .context
            .propagator()
            .fields()
            .map(variable)
            .collect();
        let env = |name: &str| {
            propagators(name).or_else(|| asked.contains(name).then(|| OsString::from("x")))
        };

        let read: BTreeSet<String> = OtelEnv::read(&env)
            .context
            .carried
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        assert_eq!(read, asked, "{named:?}");
    }
}

/// A command inherits none of the known context variables, so a
/// propagator the command line serves whose variable isn't among them
/// would leave a command another span's.
#[test]
fn the_known_context_variables_cover_every_field_of_every_propagator_the_cli_serves() {
    let every = |name: &str| {
        (name == "OTEL_PROPAGATORS").then(|| OsString::from("tracecontext,baggage,b3,b3multi"))
    };
    let context = OtelEnv::read(&every).context;
    assert_eq!(context.propagators.len(), 4, "{:?}", context.propagators);

    let fields: BTreeSet<String> = context.propagator().fields().map(variable).collect();

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

    let (b3, warnings) = inbound_of(&[("OTEL_PROPAGATORS", "b3"), ("B3", &single)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(parent_of(&b3), parent);
    assert_eq!(injected(&b3), pairs(&[("B3", &single)]));

    let (b3multi, warnings) =
        inbound_of(&[&[("OTEL_PROPAGATORS", "b3multi")][..], &multiple[..]].concat());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(parent_of(&b3multi), parent);
    assert_eq!(injected(&b3multi), pairs(&multiple));

    let (b3, _) = inbound_of(&[&[("OTEL_PROPAGATORS", "b3")][..], &multiple[..]].concat());
    assert!(!b3.parent.has_active_span(), "b3 reads no X_B3_* variable");
    let (b3multi, _) = inbound_of(&[("OTEL_PROPAGATORS", "b3multi"), ("B3", &single)]);
    assert!(!b3multi.parent.has_active_span(), "b3multi reads no B3");
    let (traced, _) = inbound_of(&[("B3", &single), ("X_B3_TRACEID", TRACE_ID)]);
    assert!(
        !traced.parent.has_active_span(),
        "the default reads neither"
    );
}
