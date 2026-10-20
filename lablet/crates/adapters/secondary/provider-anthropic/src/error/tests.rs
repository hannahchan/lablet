use super::{context_exhausted, grouped, settings_named};

#[test]
fn a_byte_count_is_grouped_in_thousands() {
    assert_eq!(grouped(0), "0");
    assert_eq!(grouped(999), "999");
    assert_eq!(grouped(1_204), "1,204");
    assert_eq!(grouped(1_234_567), "1,234,567");
}

#[test]
fn the_context_window_message_is_matched_after_a_position_prefix_in_any_case() {
    assert!(context_exhausted(
        "prompt is too long: 210000 tokens > 200000 maximum"
    ));
    assert!(context_exhausted(
        "messages.12.content.0: Prompt is too long"
    ));
    assert!(!context_exhausted("messages: roles must alternate"));
    assert!(!context_exhausted("the prompt is too long"));
}

#[test]
fn the_settings_a_message_names_are_read_off_its_complaint_and_not_its_remedy() {
    assert_eq!(
        settings_named(
            "\"thinking.type.enabled\" is not supported for this model. Use \
             \"thinking.type.adaptive\" and \"output_config.effort\" to control thinking behavior.",
            false
        )
        .as_deref(),
        Some("; the setting is model.thinking")
    );
}

#[test]
fn the_settings_a_message_names_are_appended_in_the_table_s_order() {
    assert_eq!(
        settings_named(
            "output_config.effort 'xhigh' is not supported when thinking is disabled on this model",
            false
        )
        .as_deref(),
        Some("; the settings are model.thinking and model.effort")
    );
    assert_eq!(
        settings_named("temperature: Extra inputs are not permitted", true).as_deref(),
        Some("; the setting is model.temperature")
    );
    assert_eq!(
        settings_named("temperature: Extra inputs are not permitted", false),
        None
    );
    assert_eq!(settings_named("messages: roles must alternate", true), None);
}
