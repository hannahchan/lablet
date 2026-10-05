use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::logs::{AnyValue, LoggerProvider as _, NoopLoggerProvider};
use opentelemetry::trace::{SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry::{Key, Value as Wire};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};

use super::*;

const TEXT: &str = "lablet.test.text";
const COUNT: &str = "lablet.test.count";
const NAMES: &str = "lablet.test.names";
const TRACE: TraceId = TraceId::from_bytes([0xab; 16]);
const SPAN: SpanId = SpanId::from_bytes([0xcd; 8]);

fn at() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_234)
}

fn context(flags: TraceFlags) -> SpanContext {
    SpanContext::new(TRACE, SPAN, flags, false, TraceState::default())
}

fn record(span: SpanContext, attributes: Vec<Attribute>) -> Record {
    Record {
        name: "lablet.test",
        severity: Severity::Warn,
        at: at(),
        span,
        attributes,
    }
}

/// A provider whose every record lands in the exporter as it's emitted.
fn exporting() -> (SdkLoggerProvider, InMemoryLogExporter) {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (provider, exporter)
}

/// The one record `exporter` received.
fn the_record(exporter: &InMemoryLogExporter) -> SdkLogRecord {
    let [only]: [SdkLogRecord; 1] = exporter
        .get_emitted_logs()
        .unwrap()
        .into_iter()
        .map(|log| log.record)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    only
}

fn on_a_span(attributes: Vec<Attribute>) -> Vec<(Key, Wire)> {
    span_attributes(attributes)
        .into_iter()
        .map(|held| (held.key, held.value))
        .collect()
}

fn on_a_record(attributes: Vec<Attribute>) -> Vec<(Key, AnyValue)> {
    let (provider, exporter) = exporting();
    record(context(TraceFlags::SAMPLED), attributes).emit_through(&provider.logger("lablet"));
    the_record(&exporter).attributes_iter().cloned().collect()
}

/// The keys of `pairs`, sorted, since their order isn't part of the contract.
fn keys<V>(pairs: Vec<(Key, V)>) -> Vec<String> {
    let mut keys: Vec<String> = pairs
        .into_iter()
        .map(|(key, _)| key.as_str().to_owned())
        .collect();
    keys.sort();
    keys
}

/// `text` as a span keeps it and as a record keeps it.
fn kept(text: String) -> (String, String) {
    let (_, on_span) = on_a_span(vec![Attribute::of(TEXT, text.clone())]).remove(0);
    let Wire::String(on_span) = on_span else {
        panic!("{on_span:?} isn't text");
    };
    let (_, on_record) = on_a_record(vec![Attribute::of(TEXT, text)]).remove(0);
    let AnyValue::String(on_record) = on_record else {
        panic!("{on_record:?} isn't text");
    };
    (on_span.as_str().to_owned(), on_record.as_str().to_owned())
}

/// Each of `texts` as a span keeps it and as a record keeps it.
fn kept_each(texts: Vec<String>) -> (Vec<String>, Vec<String>) {
    let (_, on_span) = on_a_span(vec![Attribute::of(NAMES, texts.clone())]).remove(0);
    let Wire::Array(Array::String(on_span)) = on_span else {
        panic!("{on_span:?} isn't a list of texts");
    };
    let (_, on_record) = on_a_record(vec![Attribute::of(NAMES, texts)]).remove(0);
    let AnyValue::ListAny(on_record) = on_record else {
        panic!("{on_record:?} isn't a list");
    };
    let on_record = on_record
        .into_iter()
        .map(|text| match text {
            AnyValue::String(text) => text.as_str().to_owned(),
            other => panic!("{other:?} isn't text"),
        })
        .collect();
    (
        on_span
            .into_iter()
            .map(|text| text.as_str().to_owned())
            .collect(),
        on_record,
    )
}

#[test]
fn a_value_is_made_from_each_type_the_registry_gives_an_attribute() {
    assert_eq!(Value::from("chat"), Value::Text("chat".to_owned()));
    assert_eq!(
        Value::from("bash".to_owned()),
        Value::Text("bash".to_owned())
    );
    assert_eq!(
        Value::from(vec!["bash".to_owned()]),
        Value::Texts(vec!["bash".to_owned()])
    );
    assert_eq!(Value::from(7_u64), Value::Int(7));
    assert_eq!(Value::from(u64::MAX), Value::Int(i64::MAX));
    assert_eq!(Value::from(u32::MAX), Value::Int(i64::from(u32::MAX)));
    assert_eq!(Value::from(u16::MAX), Value::Int(i64::from(u16::MAX)));
    assert_eq!(Value::from(5_u32), Value::from(5_u16));
    assert_eq!(Value::from(-4_i64), Value::Int(-4));
    assert_eq!(Value::from(0.5), Value::Float(0.5));
    assert_eq!(Value::from(true), Value::Flag(true));
}

#[test]
fn a_count_the_wire_cannot_carry_is_the_largest_it_can() {
    assert_eq!(count_of(0), 0);
    assert_eq!(count_of(7), 7);
    assert_eq!(count_of(u64::try_from(i64::MAX).unwrap()), i64::MAX);
    assert_eq!(count_of(u64::MAX), i64::MAX);
}

#[test]
fn a_template_holds_one_attribute_for_each_entry_under_its_prefix() {
    let per_tool = std::collections::BTreeMap::from([
        ("bash".to_owned(), 3_i64),
        ("read_file".to_owned(), 1_i64),
    ]);
    assert_eq!(
        each_under("lablet.test.tool", &per_tool),
        [
            Attribute::under("lablet.test.tool", "bash", 3_i64),
            Attribute::under("lablet.test.tool", "read_file", 1_i64),
        ]
    );
    let names = std::collections::BTreeMap::from([("bash".to_owned(), "sh".to_owned())]);
    assert_eq!(
        each_under("lablet.test.name", &names),
        [Attribute::under("lablet.test.name", "bash", "sh")]
    );
    let none: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    assert_eq!(
        each_under("lablet.test.tool", &none),
        Vec::<Attribute>::new()
    );
}

#[test]
fn an_attribute_reads_back_its_key_and_what_it_holds() {
    let of = Attribute::of(TEXT, "chat");
    assert_eq!(of.key(), TEXT);
    assert_eq!(of.value(), Some(&Value::Text("chat".to_owned())));

    let some = Attribute::new(COUNT, Some(Value::Int(3)));
    assert_eq!(some.key(), COUNT);
    assert_eq!(some.value(), Some(&Value::Int(3)));

    let none = Attribute::new(COUNT, None);
    assert_eq!(none.key(), COUNT);
    assert_eq!(none.value(), None);

    let under = Attribute::under("lablet.test.tool", "bash", 2_u16);
    assert_eq!(under.key(), "lablet.test.tool.bash");
    assert_eq!(under.value(), Some(&Value::Int(2)));
}

#[test]
fn a_text_as_long_as_the_limit_is_kept_whole_on_a_span_and_a_record() {
    let at_the_limit = "a".repeat(ATTRIBUTE_MAX_BYTES);

    let (on_span, on_record) = kept(at_the_limit.clone());

    assert_eq!(on_span, at_the_limit);
    assert_eq!(on_record, at_the_limit);
}

#[test]
fn a_text_one_byte_over_the_limit_is_cut_to_it_on_a_span_and_a_record() {
    let (on_span, on_record) = kept("a".repeat(ATTRIBUTE_MAX_BYTES + 1));

    assert_eq!(on_span, "a".repeat(ATTRIBUTE_MAX_BYTES));
    assert_eq!(on_record, on_span);
}

#[test]
fn a_text_is_cut_at_a_character_boundary_on_a_span_and_a_record() {
    // Three bytes to a character, so the limit falls inside one.
    let (on_span, on_record) = kept("€".repeat(ATTRIBUTE_MAX_BYTES / 3 + 1));

    assert_eq!(on_span, "€".repeat(ATTRIBUTE_MAX_BYTES / 3));
    assert_eq!(on_span.len(), ATTRIBUTE_MAX_BYTES - ATTRIBUTE_MAX_BYTES % 3);
    assert_eq!(on_record, on_span);
}

#[test]
fn each_text_of_a_list_is_cut_on_its_own_on_a_span_and_a_record() {
    let texts = vec![
        "a".repeat(ATTRIBUTE_MAX_BYTES),
        "b".repeat(ATTRIBUTE_MAX_BYTES + 1),
        "€".repeat(ATTRIBUTE_MAX_BYTES / 3 + 1),
        "short".to_owned(),
    ];

    let (on_span, on_record) = kept_each(texts);

    assert_eq!(
        on_span,
        [
            "a".repeat(ATTRIBUTE_MAX_BYTES),
            "b".repeat(ATTRIBUTE_MAX_BYTES),
            "€".repeat(ATTRIBUTE_MAX_BYTES / 3),
            "short".to_owned(),
        ]
    );
    assert_eq!(on_record, on_span);
}

#[test]
fn an_attribute_without_a_value_is_left_out_and_its_neighbours_are_kept() {
    let attributes = || {
        vec![
            Attribute::of(COUNT, 1_u32),
            Attribute::new(TEXT, None),
            Attribute::of(NAMES, vec!["read_file".to_owned()]),
        ]
    };

    assert_eq!(keys(on_a_span(attributes())), [COUNT, NAMES]);
    assert_eq!(keys(on_a_record(attributes())), [COUNT, NAMES]);
}

#[test]
fn a_span_and_a_record_are_given_the_same_values() {
    let attributes = || {
        vec![
            Attribute::of(TEXT, "chat"),
            Attribute::of(COUNT, 2_u16),
            Attribute::of("lablet.test.ratio", 0.5),
            Attribute::of("lablet.test.flag", true),
            Attribute::of("lablet.test.seed", -4_i64),
            Attribute::of(NAMES, vec!["bash".to_owned()]),
            Attribute::under("lablet.test.tool", "bash", 3_u64),
        ]
    };

    assert_eq!(
        on_a_span(attributes()),
        [
            (Key::new(TEXT), Wire::from("chat")),
            (Key::new(COUNT), Wire::I64(2)),
            (Key::new("lablet.test.ratio"), Wire::F64(0.5)),
            (Key::new("lablet.test.flag"), Wire::Bool(true)),
            (Key::new("lablet.test.seed"), Wire::I64(-4)),
            (
                Key::new(NAMES),
                Wire::Array(Array::String(vec!["bash".into()]))
            ),
            (Key::new("lablet.test.tool.bash"), Wire::I64(3)),
        ]
    );
    assert_eq!(
        on_a_record(attributes()),
        [
            (Key::new(TEXT), AnyValue::from("chat")),
            (Key::new(COUNT), AnyValue::Int(2)),
            (Key::new("lablet.test.ratio"), AnyValue::Double(0.5)),
            (Key::new("lablet.test.flag"), AnyValue::Boolean(true)),
            (Key::new("lablet.test.seed"), AnyValue::Int(-4)),
            (
                Key::new(NAMES),
                AnyValue::ListAny(Box::new(vec![AnyValue::from("bash")]))
            ),
            (Key::new("lablet.test.tool.bash"), AnyValue::Int(3)),
        ]
    );
}

#[test]
fn a_record_reaches_the_exporter_as_it_was_made() {
    let (provider, exporter) = exporting();

    record(
        context(TraceFlags::SAMPLED),
        vec![Attribute::of(COUNT, 7_u64)],
    )
    .emit_through(&provider.logger("lablet"));

    let received = the_record(&exporter);
    assert_eq!(received.event_name(), Some("lablet.test"));
    assert_eq!(received.severity_number(), Some(Severity::Warn));
    assert_eq!(received.severity_text(), Some("WARN"));
    assert_eq!(received.timestamp(), Some(at()));
    assert_eq!(received.observed_timestamp(), Some(at()));
    let context = received.trace_context().unwrap();
    assert_eq!(context.trace_id, TRACE);
    assert_eq!(context.span_id, SPAN);
    assert_eq!(context.trace_flags, Some(TraceFlags::SAMPLED));
    assert_eq!(
        received.attributes_iter().cloned().collect::<Vec<_>>(),
        [(Key::new(COUNT), AnyValue::Int(7))]
    );
}

#[test]
fn a_bridge_is_lablets_logger_over_a_logger_of_the_api() {
    let (provider, exporter) = exporting();
    let logger: Box<dyn Logger> = Box::new(Bridge::new(provider.logger("lablet")));

    logger.emit(record(
        context(TraceFlags::default()),
        vec![Attribute::of(TEXT, "through the bridge")],
    ));

    let received = the_record(&exporter);
    assert_eq!(received.event_name(), Some("lablet.test"));
    assert_eq!(received.timestamp(), Some(at()));
    assert_eq!(
        received.trace_context().unwrap().trace_flags,
        Some(TraceFlags::default())
    );
    assert_eq!(
        received.attributes_iter().cloned().collect::<Vec<_>>(),
        [(Key::new(TEXT), AnyValue::from("through the bridge"))]
    );
}

// The no-op logger has nothing to read back, so this holds only that a bridge
// over it is lablet's logger, which the `Bridge` doc tells a test to use.
#[test]
fn a_bridge_over_the_apis_no_op_logger_is_lablets_logger_too() {
    let logger: Box<dyn Logger> = Box::new(Bridge::new(NoopLoggerProvider::new().logger("lablet")));

    logger.emit(record(
        context(TraceFlags::SAMPLED),
        vec![Attribute::of(TEXT, "dropped")],
    ));
}

// The generated module: what it fixes once for every signal. What each
// struct writes is held by the loop's scenarios, which record every struct
// and read it back from the SDK's in-memory exporters.

use super::generated::{
    GenAiClientInferenceOperationDetailsGenAiOperationName as Operation, Join, LabletExecuteTool,
    LabletToolStatus, SCHEMA_URL,
};

/// The loop names a tool span itself when it opens it, before the call has
/// run and the struct can be filled, so this holds that it names it as the
/// registry's template does.
#[test]
fn a_tool_span_is_named_for_its_operation_and_its_tool_as_the_loop_names_it() {
    let tool = LabletExecuteTool {
        join: Join {
            gen_ai_conversation_id: "01J9Z3".to_owned(),
            lablet_config_digest: "9f86d081".to_owned(),
            session_id: "01J9Z3".to_owned(),
            lablet_experiment_id: None,
            lablet_task_id: None,
            lablet_trial: None,
        },
        gen_ai_tool_call_id: "call_1".to_owned(),
        gen_ai_tool_name: "bash".to_owned(),
        lablet_tool_input_bytes: 12,
        lablet_tool_is_error: false,
        lablet_tool_output_bytes: 3,
        lablet_tool_output_truncated: false,
        lablet_tool_status: LabletToolStatus::Ok,
        lablet_turn: 1,
        error_type: None,
        gen_ai_tool_description: None,
        gen_ai_tool_type: None,
        jsonrpc_request_id: None,
        lablet_tool_output_original_bytes: None,
        lablet_tool_source: None,
        mcp_method_name: None,
        mcp_protocol_version: None,
        mcp_session_id: None,
        network_transport: None,
        rpc_response_status_code: None,
    };

    assert_eq!(
        tool.name(),
        format!("{} bash", LabletExecuteTool::GEN_AI_OPERATION_NAME)
    );
    assert_eq!(tool.name(), "execute_tool bash");
}

#[test]
fn the_module_carries_the_registrys_schema_url() {
    assert_eq!(SCHEMA_URL, "https://lablet.dev/schemas/0.1.0");
}

#[test]
fn the_content_records_operations_are_the_three_spans() {
    assert_eq!(
        Operation::ALL.map(Operation::as_str),
        ["invoke_agent", "chat", "execute_tool"]
    );
    for operation in Operation::ALL {
        assert_eq!(
            Value::from(operation),
            Value::Text(operation.as_str().to_owned())
        );
    }
}
