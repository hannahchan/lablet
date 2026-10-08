//! The canaries of the gaps lablet fills in B3's propagator.

use std::collections::HashMap;

use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry_propagator_b3::{B3Encoding, Propagator};

/// The command line's `b3` injects only the keys its fields name, `b3`
/// alone, since the specification's `b3` is the single header. When the
/// crate's single header encoding stops injecting the multiple headers,
/// this fails, and the filter around it can go.
#[test]
fn canary_b3_s_single_header_encoding_injects_the_multiple_headers_too() {
    let propagator = Propagator::with_encoding(B3Encoding::SingleHeader);
    let context = Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
        SpanId::from_hex("b7ad6b7169203331").unwrap(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    ));

    let mut injected = HashMap::new();
    propagator.inject_context(&context, &mut injected);

    assert_eq!(propagator.fields().collect::<Vec<_>>(), ["b3"]);
    let mut keys: Vec<&str> = injected.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["b3", "x-b3-sampled", "x-b3-spanid", "x-b3-traceid"]);
}

/// The command line's `b3` and `b3multi` inject nothing for a context with
/// no valid span. When the crate stops injecting a sampled flag for one,
/// this fails, and the check in front of it can go.
#[test]
fn canary_b3_injects_a_sampled_flag_for_a_context_with_no_valid_span() {
    for (encoding, keys) in [
        (B3Encoding::SingleHeader, &["b3", "x-b3-sampled"][..]),
        (B3Encoding::MultipleHeader, &["x-b3-sampled"][..]),
    ] {
        let mut injected = HashMap::new();
        Propagator::with_encoding(encoding).inject_context(&Context::new(), &mut injected);

        let flags: HashMap<String, String> = keys
            .iter()
            .map(|&key| (key.to_owned(), "0".to_owned()))
            .collect();
        assert_eq!(injected, flags);
    }
}
