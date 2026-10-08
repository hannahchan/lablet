use super::*;

const SAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn environment(held: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    held.iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

/// The carriers specification's normalisation, with its own example.
#[test]
fn a_key_is_carried_by_the_variable_its_normalised_name_names() {
    assert_eq!(variable("traceparent"), "TRACEPARENT");
    assert_eq!(variable("x-b3-traceid"), "X_B3_TRACEID");
    assert_eq!(variable("Ünïcode.key"), "_N_CODE_KEY");
    assert_eq!(variable("1st"), "_1ST");
    assert_eq!(variable("snake_case9"), "SNAKE_CASE9");
    assert_eq!(variable(""), "_");
}

#[test]
fn the_extractor_reads_a_key_from_its_normalised_variable_and_an_empty_one_as_none() {
    let held = environment(&[
        ("TRACEPARENT", SAMPLED),
        ("TRACESTATE", ""),
        ("traceparent", "a variable named as the key is written"),
    ]);
    let carrier = EnvExtractor(&held);

    assert_eq!(carrier.get("traceparent"), Some(SAMPLED));
    assert_eq!(carrier.get("TraceParent"), Some(SAMPLED));
    assert_eq!(carrier.get("tracestate"), None);
    assert_eq!(carrier.get("baggage"), None);
    assert_eq!(carrier.keys(), ["TRACEPARENT", "TRACESTATE", "traceparent"]);
}

#[test]
fn a_value_that_is_no_text_is_not_extracted() {
    use std::os::unix::ffi::OsStringExt as _;
    let mut held = BTreeMap::new();
    held.insert(
        OsString::from("TRACEPARENT"),
        OsString::from_vec(b"00-\xFF".to_vec()),
    );
    held.insert(OsString::from_vec(b"NOT_\xFF_TEXT".to_vec()), "1".into());

    let carrier = EnvExtractor(&held);
    assert_eq!(carrier.get("traceparent"), None);
    assert_eq!(carrier.keys(), ["TRACEPARENT"]);
}

#[test]
fn the_injector_writes_each_key_as_its_normalised_variable() {
    let mut written = environment(&[("PATH", "/usr/bin"), ("TRACESTATE", "inherited=1")]);
    let stated = BTreeSet::new();
    let mut carrier = EnvInjector::new(&mut written, &stated);

    carrier.set("traceparent", SAMPLED.to_owned());
    carrier.set(
        "x-b3-traceid",
        "0af7651916cd43dd8448eb211c80319c".to_owned(),
    );
    carrier.set("baggage", String::new());
    carrier.set("tracestate", String::new());

    assert_eq!(
        written,
        environment(&[
            ("PATH", "/usr/bin"),
            ("TRACEPARENT", SAMPLED),
            ("TRACESTATE", "inherited=1"),
            ("X_B3_TRACEID", "0af7651916cd43dd8448eb211c80319c"),
        ]),
        "an empty value is skipped, and leaves what was there"
    );
}

#[test]
fn the_injector_leaves_a_variable_the_config_states() {
    let mut written = environment(&[("TRACEPARENT", "stated")]);
    let stated = BTreeSet::from(["TRACEPARENT".to_owned()]);
    let mut carrier = EnvInjector::new(&mut written, &stated);

    carrier.set("traceparent", SAMPLED.to_owned());
    carrier.set("tracestate", "injected=1".to_owned());

    assert_eq!(
        written,
        environment(&[("TRACEPARENT", "stated"), ("TRACESTATE", "injected=1")])
    );
}

#[test]
fn every_context_variable_is_its_own_normalised_name() {
    for name in CONTEXT_VARIABLES {
        assert_eq!(variable(name), name);
    }
}
