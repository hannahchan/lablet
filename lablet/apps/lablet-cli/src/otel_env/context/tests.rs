//! The resource's and the inbound context's variables, read over
//! environments the tests state.

use super::*;
use crate::otel_env::tests::warned;

fn read(held: &[(&str, &str)]) -> (Context, Vec<String>) {
    warned(held, Context::read)
}

const BOTH: [Propagator; 2] = [Propagator::TraceContext, Propagator::Baggage];

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn carried(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn an_environment_that_sets_nothing_gives_no_service_name_no_attributes_and_both_propagators() {
    let (context, warnings) = read(&[]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(context, Context::default());
    assert_eq!(context.service_name, None);
    assert!(context.resource_attributes.is_empty());
    assert_eq!(context.propagators, BOTH);
    assert!(context.carried.is_empty());
}

#[test]
fn an_empty_service_name_or_resource_attributes_variable_is_unset() {
    let (context, warnings) = read(&[(SERVICE_NAME, ""), (RESOURCE_ATTRIBUTES, "")]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(context.service_name, None);
    assert!(context.resource_attributes.is_empty());
}

#[test]
fn otel_service_name_is_read_as_it_is_written() {
    let (context, _) = read(&[(SERVICE_NAME, "checkout %41")]);

    assert_eq!(context.service_name.as_deref(), Some("checkout %41"));
}

/// The SDK's own detector trims each side and splits on the first `=`,
/// and never decodes; lablet decodes each side once, after trimming, so an
/// escaped space at either end stays.
#[test]
fn resource_attribute_keys_and_values_are_percent_decoded_once() {
    let (context, warnings) = read(&[(
        RESOURCE_ATTRIBUTES,
        " team = a%2Cb ,deploy%3Dment=x%3Dy,%20edge%20=%e2%9c%93,double=%2520,",
    )]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        context.resource_attributes,
        [
            ("team".to_owned(), "a,b".to_owned()),
            ("deploy=ment".to_owned(), "x=y".to_owned()),
            (" edge ".to_owned(), "\u{2713}".to_owned()),
            ("double".to_owned(), "%20".to_owned()),
        ]
    );
}

#[test]
fn a_resource_attributes_value_that_does_not_decode_is_discarded_whole_with_a_warning() {
    for broken in [
        "team=evals,secret=a%2",
        "team=evals,secret=a%zz",
        "team=evals,secret=%ff",
        "team=evals,sec%g1ret=a",
        "team=evals,secret=a%+1",
    ] {
        let (context, warnings) = read(&[(RESOURCE_ATTRIBUTES, broken)]);

        assert!(context.resource_attributes.is_empty(), "{broken}");
        assert_eq!(
            warnings,
            [format!(
                "`{RESOURCE_ATTRIBUTES}` holds a value that doesn't percent-decode, so the \
                 whole of it is ignored"
            )],
            "{broken}"
        );
    }
}

/// The resource specification discards the whole value on any error, so
/// a pair that isn't one takes the pairs beside it with it, whether or not
/// it decodes. A blank member isn't a pair, and is skipped.
#[test]
fn a_resource_attributes_pair_with_no_equals_sign_or_no_key_discards_the_whole_value() {
    for (broken, why) in [
        ("team=evals,lonely", "has a pair with no `=`"),
        ("team=evals,junk%ZZ", "has a pair with no `=`"),
        ("team=evals,=orphan", "has a pair with no key"),
        ("team=evals, =orphan", "has a pair with no key"),
    ] {
        let (context, warnings) = read(&[(RESOURCE_ATTRIBUTES, broken)]);

        assert!(context.resource_attributes.is_empty(), "{broken}");
        assert_eq!(
            warnings,
            [format!(
                "`{RESOURCE_ATTRIBUTES}` holds a value that {why}, so the whole of it is ignored"
            )],
            "{broken}"
        );
    }

    let (context, warnings) = read(&[(RESOURCE_ATTRIBUTES, "team=evals,, ,tier=gold,")]);
    assert_eq!(
        context.resource_attributes,
        [
            ("team".to_owned(), "evals".to_owned()),
            ("tier".to_owned(), "gold".to_owned()),
        ]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn the_context_variables_are_read_by_their_normalised_names() {
    let (context, warnings) = read(&[
        ("TRACEPARENT", TRACEPARENT),
        ("TRACESTATE", "vendor=1"),
        ("BAGGAGE", "user=1"),
    ]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        context.carried,
        carried(&[
            ("BAGGAGE", "user=1"),
            ("TRACEPARENT", TRACEPARENT),
            ("TRACESTATE", "vendor=1"),
        ])
    );
}

/// The carriers specification: a variable whose name isn't normalised is
/// never read, even when it normalises to one a propagator asks for.
#[test]
fn a_lower_case_traceparent_variable_is_not_read() {
    let (context, _) = read(&[
        ("traceparent", TRACEPARENT),
        ("tracestate", "vendor=1"),
        ("baggage", "user=1"),
    ]);

    assert!(context.carried.is_empty(), "{:?}", context.carried);
}

#[test]
fn otel_propagators_is_read_in_any_case_with_duplicates_dropped() {
    let (context, warnings) = read(&[(PROPAGATORS, " Baggage ,TRACECONTEXT,baggage")]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        context.propagators,
        [Propagator::Baggage, Propagator::TraceContext]
    );
}

#[test]
fn otel_propagators_none_ignores_the_context_variables() {
    let (context, warnings) = read(&[
        (PROPAGATORS, "NONE"),
        ("TRACEPARENT", TRACEPARENT),
        ("BAGGAGE", "user=1"),
    ]);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(context.propagators.is_empty());
    assert!(context.carried.is_empty(), "{:?}", context.carried);
}

#[test]
fn an_unknown_propagator_is_ignored_with_a_warning() {
    let (context, warnings) = read(&[(PROPAGATORS, "xray,tracecontext,none")]);

    assert_eq!(context.propagators, [Propagator::TraceContext]);
    assert_eq!(
        warnings,
        [
            format!("`{PROPAGATORS}` holds `xray`, which isn't one lablet serves, so it's ignored"),
            format!("`{PROPAGATORS}` holds `none`, which is beside another value, so it's ignored"),
        ]
    );
}

/// The specification's MUST is to ignore what isn't recognised, so a list
/// of nothing lablet serves is as if it weren't set, rather than `none`.
#[test]
fn a_propagator_list_naming_nothing_lablet_serves_is_read_as_unset() {
    let (context, warnings) = read(&[(PROPAGATORS, "jaeger,xray"), ("TRACEPARENT", TRACEPARENT)]);

    assert_eq!(context.propagators, BOTH);
    assert_eq!(context.carried, carried(&[("TRACEPARENT", TRACEPARENT)]));
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

/// Each propagator's variables are read, and only theirs: the context
/// variables are those the composite's fields name, normalised.
#[test]
fn b3_and_b3multi_are_served_and_read_the_variables_their_fields_name() {
    let held = [
        ("TRACEPARENT", TRACEPARENT),
        ("B3", "single"),
        ("X_B3_TRACEID", "trace"),
        ("X_B3_SPANID", "span"),
        ("X_B3_PARENTSPANID", "parent"),
        ("X_B3_SAMPLED", "1"),
        ("X_B3_FLAGS", "0"),
    ];

    let (single, warnings) = read(&[&held[..], &[(PROPAGATORS, "b3")]].concat());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(single.propagators, [Propagator::B3]);
    assert_eq!(single.carried, carried(&[("B3", "single")]));

    let (multiple, _) = read(&[&held[..], &[(PROPAGATORS, "B3Multi")]].concat());
    assert_eq!(multiple.propagators, [Propagator::B3Multi]);
    assert_eq!(
        multiple.carried,
        carried(&[
            ("X_B3_FLAGS", "0"),
            ("X_B3_SAMPLED", "1"),
            ("X_B3_SPANID", "span"),
            ("X_B3_TRACEID", "trace"),
        ])
    );

    let (all, _) = read(&[&held[..], &[(PROPAGATORS, "b3multi,tracecontext,b3")]].concat());
    assert_eq!(
        all.propagators,
        [
            Propagator::B3Multi,
            Propagator::TraceContext,
            Propagator::B3
        ]
    );
    assert_eq!(
        all.carried,
        carried(&[
            ("B3", "single"),
            ("TRACEPARENT", TRACEPARENT),
            ("X_B3_FLAGS", "0"),
            ("X_B3_SAMPLED", "1"),
            ("X_B3_SPANID", "span"),
            ("X_B3_TRACEID", "trace"),
        ])
    );
}

/// Under `OTEL_SDK_DISABLED=true` with no inbound context, a tool span has
/// no valid context, and a command gets no B3 variable at all.
#[test]
fn b3_and_b3multi_inject_nothing_for_a_context_with_no_valid_span() {
    for propagator in [Propagator::B3, Propagator::B3Multi] {
        let mut injected = std::collections::HashMap::new();
        composite(&[propagator]).inject_context(&OtelContext::new(), &mut injected);

        assert!(injected.is_empty(), "{propagator:?}: {injected:?}");
    }
}
