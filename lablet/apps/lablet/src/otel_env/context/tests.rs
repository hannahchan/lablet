//! The resource's and the inbound context's variables, read over
//! environments the tests state.

use super::*;
use crate::otel_env::tests::warned;

fn read(held: &[(&str, &str)]) -> (Context, Vec<String>) {
    warned(held, Context::read)
}

const BOTH: [Propagator; 2] = [Propagator::TraceContext, Propagator::Baggage];

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

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

/// The SDK ignores such a pair without a word; the seam says so, naming
/// the variable alone.
#[test]
fn a_resource_attributes_pair_with_no_equals_sign_or_no_key_is_ignored_with_a_warning() {
    let (context, warnings) = read(&[(RESOURCE_ATTRIBUTES, "team=evals,lonely,=orphan")]);

    assert_eq!(
        context.resource_attributes,
        [("team".to_owned(), "evals".to_owned())]
    );
    assert_eq!(
        warnings,
        [
            format!(
                "`{RESOURCE_ATTRIBUTES}` holds a value that has a pair with no `=`, which is ignored"
            ),
            format!(
                "`{RESOURCE_ATTRIBUTES}` holds a value that has a pair with no key, which is ignored"
            ),
        ]
    );
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
        [
            ("TRACEPARENT", TRACEPARENT.to_owned()),
            ("TRACESTATE", "vendor=1".to_owned()),
            ("BAGGAGE", "user=1".to_owned()),
        ]
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
    let (context, warnings) = read(&[(PROPAGATORS, "b3,tracecontext,none")]);

    assert_eq!(context.propagators, [Propagator::TraceContext]);
    assert_eq!(
        warnings,
        [
            format!("`{PROPAGATORS}` holds `b3`, which isn't one lablet serves, so it's ignored"),
            format!("`{PROPAGATORS}` holds `none`, which is beside another value, so it's ignored"),
        ]
    );
}

/// The specification's MUST is to ignore what isn't recognised, so a list
/// of nothing lablet serves is as if it weren't set, rather than `none`.
#[test]
fn a_propagator_list_naming_nothing_lablet_serves_is_read_as_unset() {
    let (context, warnings) = read(&[(PROPAGATORS, "b3multi,xray"), ("TRACEPARENT", TRACEPARENT)]);

    assert_eq!(context.propagators, BOTH);
    assert_eq!(context.carried, [("TRACEPARENT", TRACEPARENT.to_owned())]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}
