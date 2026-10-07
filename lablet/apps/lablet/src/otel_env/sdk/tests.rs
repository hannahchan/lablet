//! The SDK's variables, read over environments the tests state.

use super::*;
use crate::otel_env::tests::warned;

fn read(held: &[(&str, &str)]) -> (Sdk, Vec<String>) {
    warned(held, Sdk::read)
}

const ALWAYS_ON: Sampling = Sampling {
    parent_based: true,
    root: Root::AlwaysOn,
};

#[test]
fn an_environment_that_sets_nothing_gives_the_specifications_defaults() {
    let (sdk, warnings) = read(&[]);

    assert!(warnings.is_empty());
    assert_eq!(sdk, Sdk::default());
    assert_eq!(sdk.sampling, ALWAYS_ON);
    assert_eq!(
        sdk.limits,
        Limits {
            attributes_per_span: 128,
            events_per_span: 128,
            links_per_span: 128,
            attributes_per_event: 128,
            attributes_per_link: 128,
        }
    );
    assert_eq!(
        (sdk.spans, sdk.logs),
        (
            Batch {
                schedule_delay: Duration::from_secs(5),
                max_queue_size: 2_048,
                max_export_batch_size: 512,
            },
            Batch {
                schedule_delay: Duration::from_secs(1),
                max_queue_size: 2_048,
                max_export_batch_size: 512,
            }
        )
    );
}

#[test]
fn a_sampler_is_read_in_any_case() {
    let cases = [
        ("Always_On", false, Root::AlwaysOn),
        ("ALWAYS_OFF", false, Root::AlwaysOff),
        ("TraceIdRatio", false, Root::TraceIdRatio(0.25)),
        ("parentbased_always_on", true, Root::AlwaysOn),
        ("ParentBased_Always_Off", true, Root::AlwaysOff),
        ("PARENTBASED_TRACEIDRATIO", true, Root::TraceIdRatio(0.25)),
    ];
    for (name, parent_based, root) in cases {
        let (sdk, warnings) = read(&[
            ("OTEL_TRACES_SAMPLER", name),
            ("OTEL_TRACES_SAMPLER_ARG", "0.25"),
        ]);

        assert_eq!(sdk.sampling, Sampling { parent_based, root }, "{name}");
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
    }
}

#[test]
fn a_sampler_lablet_does_not_build_is_ignored_with_a_warning() {
    for name in [
        "jaeger_remote",
        "parentbased_jaeger_remote",
        "xray",
        "sometimes",
    ] {
        let (sdk, warnings) = read(&[("OTEL_TRACES_SAMPLER", name)]);

        assert_eq!(sdk.sampling, ALWAYS_ON, "{name}");
        assert_eq!(
            warnings,
            [format!(
                "`OTEL_TRACES_SAMPLER` holds `{name}`, which isn't one lablet serves, so it's \
                 ignored"
            )]
        );
    }
}

#[test]
fn a_sampler_argument_that_does_not_parse_is_read_as_unset_with_a_warning() {
    for argument in ["half", "1.5", "-0.1", "NaN"] {
        let (sdk, warnings) = read(&[
            ("OTEL_TRACES_SAMPLER", "traceidratio"),
            ("OTEL_TRACES_SAMPLER_ARG", argument),
        ]);

        assert_eq!(sdk.sampling.root, Root::TraceIdRatio(1.0), "{argument}");
        assert_eq!(
            warnings,
            [format!(
                "`OTEL_TRACES_SAMPLER_ARG` holds `{argument}`, which isn't a ratio from 0 to 1, \
                 so it's read as unset, which is 1"
            )]
        );
    }
    let (sdk, warnings) = read(&[("OTEL_TRACES_SAMPLER", "traceidratio")]);
    assert_eq!(sdk.sampling.root, Root::TraceIdRatio(1.0));
    assert!(warnings.is_empty(), "an argument left unset says nothing");
}

#[test]
fn a_sampler_argument_is_read_only_for_a_sampler_that_takes_one() {
    for held in [
        &[("OTEL_TRACES_SAMPLER_ARG", "half")][..],
        &[
            ("OTEL_TRACES_SAMPLER", "always_off"),
            ("OTEL_TRACES_SAMPLER_ARG", "half"),
        ],
    ] {
        let (_, warnings) = read(held);

        assert!(warnings.is_empty(), "{held:?}: {warnings:?}");
    }
}

#[test]
fn the_sdks_sampler_is_the_one_the_environment_names() {
    let sampler = |held| format!("{:?}", read(held).0.sampling.sampler());

    assert_eq!(sampler(&[]), "ParentBased(AlwaysOn)");
    assert_eq!(
        sampler(&[("OTEL_TRACES_SAMPLER", "always_off")]),
        "AlwaysOff"
    );
    assert_eq!(
        sampler(&[
            ("OTEL_TRACES_SAMPLER", "parentbased_traceidratio"),
            ("OTEL_TRACES_SAMPLER_ARG", "0.5"),
        ]),
        "ParentBased(TraceIdRatioBased(0.5))"
    );
}

#[test]
fn the_attribute_count_limit_stands_beneath_the_span_ones() {
    let (beneath, _) = read(&[("OTEL_ATTRIBUTE_COUNT_LIMIT", "7")]);
    let (over, _) = read(&[
        ("OTEL_ATTRIBUTE_COUNT_LIMIT", "7"),
        ("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT", "2"),
        ("OTEL_LINK_ATTRIBUTE_COUNT_LIMIT", "3"),
        ("OTEL_SPAN_EVENT_COUNT_LIMIT", "4"),
        ("OTEL_SPAN_LINK_COUNT_LIMIT", "0"),
    ]);

    assert_eq!(
        beneath.limits,
        Limits {
            attributes_per_span: 7,
            events_per_span: 128,
            links_per_span: 128,
            attributes_per_event: 7,
            attributes_per_link: 7,
        },
        "the three attribute counts, and not the counts of events and links"
    );
    assert_eq!(
        over.limits,
        Limits {
            attributes_per_span: 1,
            events_per_span: 4,
            links_per_span: 0,
            attributes_per_event: 2,
            attributes_per_link: 3,
        }
    );
}

#[test]
fn the_sdks_span_limits_are_the_ones_the_environment_names() {
    let (sdk, _) = read(&[
        ("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "1"),
        ("OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT", "2"),
        ("OTEL_LINK_ATTRIBUTE_COUNT_LIMIT", "3"),
        ("OTEL_SPAN_EVENT_COUNT_LIMIT", "4"),
        ("OTEL_SPAN_LINK_COUNT_LIMIT", "5"),
    ]);

    let SpanLimits {
        max_events_per_span,
        max_attributes_per_span,
        max_links_per_span,
        max_attributes_per_event,
        max_attributes_per_link,
    } = sdk.limits.span_limits();
    assert_eq!(
        [
            max_attributes_per_span,
            max_attributes_per_event,
            max_attributes_per_link,
            max_events_per_span,
            max_links_per_span,
        ],
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn a_limit_or_batch_field_that_is_not_a_whole_number_is_ignored_with_a_warning() {
    let (sdk, warnings) = read(&[
        ("OTEL_SPAN_EVENT_COUNT_LIMIT", "lots"),
        ("OTEL_ATTRIBUTE_COUNT_LIMIT", "-1"),
        ("OTEL_BSP_SCHEDULE_DELAY", "5s"),
        ("OTEL_BSP_MAX_QUEUE_SIZE", "0"),
        ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "1.5"),
    ]);

    assert_eq!(sdk, Sdk::default());
    assert_eq!(
        warnings,
        [
            "`OTEL_ATTRIBUTE_COUNT_LIMIT` holds `-1`, which isn't a whole number, so it's ignored",
            "`OTEL_SPAN_EVENT_COUNT_LIMIT` holds `lots`, which isn't a whole number, so it's \
             ignored",
            "`OTEL_BSP_SCHEDULE_DELAY` holds `5s`, which isn't a whole number of milliseconds, so \
             it's ignored",
            "`OTEL_BSP_MAX_QUEUE_SIZE` holds `0`, which isn't a whole number above zero, so it's \
             ignored",
            "`OTEL_BLRP_MAX_EXPORT_BATCH_SIZE` holds `1.5`, which isn't a whole number above \
             zero, so it's ignored",
        ]
    );
}

#[test]
fn the_batch_fields_the_environment_names_reach_each_processors_config() {
    let (sdk, warnings) = read(&[
        ("OTEL_BSP_SCHEDULE_DELAY", "250"),
        ("OTEL_BSP_MAX_QUEUE_SIZE", "64"),
        ("OTEL_BSP_MAX_EXPORT_BATCH_SIZE", "8"),
        ("OTEL_BLRP_SCHEDULE_DELAY", "0"),
        ("OTEL_BLRP_MAX_QUEUE_SIZE", "32"),
        ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "4"),
    ]);

    assert!(warnings.is_empty());
    assert_eq!(
        (sdk.spans, sdk.logs),
        (
            Batch {
                schedule_delay: Duration::from_millis(250),
                max_queue_size: 64,
                max_export_batch_size: 8,
            },
            Batch {
                schedule_delay: Duration::ZERO,
                max_queue_size: 32,
                max_export_batch_size: 4,
            }
        )
    );
    let spans = format!("{:?}", sdk.spans.span_config());
    let logs = format!("{:?}", sdk.logs.log_config());
    for (config, expected) in [
        (
            &spans,
            "max_queue_size: 64, scheduled_delay: 250ms, max_export_batch_size: 8",
        ),
        (
            &logs,
            "max_queue_size: 32, scheduled_delay: 0ns, max_export_batch_size: 4",
        ),
    ] {
        assert!(config.contains(expected), "{config}");
    }
}

#[test]
fn the_batch_fields_are_taken_as_given_up_to_the_specifications_largest_integer() {
    // A processor built from these allocates the whole queue and spins at
    // the zero delay, and lablet takes both anyway, as decided on
    // 2026-10-07, so a cap or a refusal here is a change of that decision.
    let largest = "2147483647";
    let (sdk, warnings) = read(&[
        ("OTEL_BSP_SCHEDULE_DELAY", "0"),
        ("OTEL_BSP_MAX_QUEUE_SIZE", largest),
        ("OTEL_BSP_MAX_EXPORT_BATCH_SIZE", largest),
        ("OTEL_BLRP_SCHEDULE_DELAY", "0"),
        ("OTEL_BLRP_MAX_QUEUE_SIZE", largest),
        ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", largest),
    ]);

    assert!(warnings.is_empty(), "{warnings:?}");
    let batch = Batch {
        schedule_delay: Duration::ZERO,
        max_queue_size: 2_147_483_647,
        max_export_batch_size: 2_147_483_647,
    };
    assert_eq!((sdk.spans, sdk.logs), (batch, batch));
    for config in [
        format!("{:?}", sdk.spans.span_config()),
        format!("{:?}", sdk.logs.log_config()),
    ] {
        assert!(
            config.contains(
                "max_queue_size: 2147483647, scheduled_delay: 0ns, max_export_batch_size: \
                 2147483647"
            ),
            "{config}"
        );
    }
}
