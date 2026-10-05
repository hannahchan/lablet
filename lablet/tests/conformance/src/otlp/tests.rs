use serde_json::json;

use super::*;

/// One span of a line of spans, with `fields` over what every span has.
fn span(fields: &Value) -> Value {
    let mut span = json!({
        "traceId": "abababababababababababababababab",
        "spanId": "cdcdcdcdcdcdcdcd",
        "name": "chat scripted-1",
        "kind": 3,
        "startTimeUnixNano": "1790000000005000000",
        "endTimeUnixNano": "1790000000255000000",
    });
    span.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    span
}

fn line_of_spans(spans: &[Value]) -> String {
    let line = json!({
        "resourceSpans": [{
            "resource": { "attributes": [
                { "key": "service.name", "value": { "stringValue": "lablet" } },
            ] },
            "scopeSpans": [{
                "scope": { "name": "lablet", "version": "0.1.0" },
                "schemaUrl": "https://lablet.dev/schemas/0.1.0",
                "spans": spans,
            }],
        }],
    });
    format!("{line}\n")
}

fn line_of_records(records: &[Value]) -> String {
    let line = json!({
        "resourceLogs": [{
            "resource": { "attributes": [] },
            "scopeLogs": [{ "scope": { "name": "lablet" }, "logRecords": records }],
        }],
    });
    format!("{line}\n")
}

fn fault(text: &str) -> String {
    Exported::parse(text).unwrap_err().to_string()
}

#[test]
fn no_lines_are_no_exports() {
    assert_eq!(Exported::parse(""), Ok(Exported::default()));
}

#[test]
fn each_line_is_read_as_what_its_key_says_it_exports() {
    let text = [
        line_of_spans(&[span(&json!({}))]),
        line_of_records(&[json!({ "eventName": "lablet.run" })]),
        line_of_spans(&[
            span(&json!({ "name": "execute_tool bash", "kind": 1 })),
            span(&json!({ "name": "invoke_agent lablet", "kind": 1 })),
        ]),
    ]
    .concat();

    let exported = Exported::parse(&text).unwrap();

    assert_eq!(exported.lines, 3);
    let spans: Vec<_> = exported
        .spans
        .iter()
        .map(|span| (span.line, span.name.as_str(), span.kind))
        .collect();
    assert_eq!(
        spans,
        [
            (1, "chat scripted-1", SpanKind::Client),
            (3, "execute_tool bash", SpanKind::Internal),
            (3, "invoke_agent lablet", SpanKind::Internal),
        ]
    );
    let records: Vec<_> = exported
        .records
        .iter()
        .map(|record| (record.line, record.event_name.as_str()))
        .collect();
    assert_eq!(records, [(2, "lablet.run")]);
}

#[test]
fn a_span_holds_the_resource_and_the_scope_of_its_export() {
    let exported = Exported::parse(&line_of_spans(&[span(&json!({})), span(&json!({}))])).unwrap();

    for span in &exported.spans {
        assert_eq!(
            span.resource,
            Attributes::from([("service.name".to_owned(), json!("lablet"))])
        );
        assert_eq!(
            span.scope,
            Scope {
                name: "lablet".to_owned(),
                version: "0.1.0".to_owned(),
                schema_url: "https://lablet.dev/schemas/0.1.0".to_owned(),
            }
        );
    }
}

#[test]
fn a_span_is_read_with_its_ids_its_times_its_events_and_its_status() {
    let exported = Exported::parse(&line_of_spans(&[span(&json!({
        "parentSpanId": "efefefefefefefef",
        "flags": 257,
        "attributes": [{ "key": "lablet.turn", "value": { "intValue": "2" } }],
        "events": [{
            "timeUnixNano": "1790000000255000000",
            "name": "lablet.retry",
            "attributes": [{ "key": "lablet.retry.will_retry", "value": { "boolValue": true } }],
        }],
        "status": { "message": "529 overloaded", "code": 2 },
    }))]))
    .unwrap();

    assert_eq!(
        exported.spans,
        [Span {
            line: 1,
            resource: Attributes::from([("service.name".to_owned(), json!("lablet"))]),
            scope: Scope {
                name: "lablet".to_owned(),
                version: "0.1.0".to_owned(),
                schema_url: "https://lablet.dev/schemas/0.1.0".to_owned(),
            },
            trace_id: "abababababababababababababababab".to_owned(),
            span_id: "cdcdcdcdcdcdcdcd".to_owned(),
            parent_span_id: Some("efefefefefefefef".to_owned()),
            flags: 257,
            name: "chat scripted-1".to_owned(),
            kind: SpanKind::Client,
            start_unix_nano: 1_790_000_000_005_000_000,
            end_unix_nano: 1_790_000_000_255_000_000,
            attributes: Attributes::from([("lablet.turn".to_owned(), json!(2))]),
            events: vec![SpanEvent {
                name: "lablet.retry".to_owned(),
                time_unix_nano: 1_790_000_000_255_000_000,
                attributes: Attributes::from([("lablet.retry.will_retry".to_owned(), json!(true))]),
            }],
            status: Status::Error("529 overloaded".to_owned()),
        }]
    );
    assert_eq!(exported.spans[0].duration_ms(), 250);
}

#[test]
fn a_root_span_has_no_parent_and_a_span_that_says_nothing_of_its_end_has_no_status() {
    let exported = Exported::parse(&line_of_spans(&[
        span(&json!({})),
        span(&json!({ "parentSpanId": "", "status": {} })),
        span(&json!({ "status": { "code": 1 } })),
    ]))
    .unwrap();

    let read: Vec<_> = exported
        .spans
        .iter()
        .map(|span| (span.parent_span_id.clone(), span.status.clone()))
        .collect();
    assert_eq!(
        read,
        [
            (None, Status::Unset),
            (None, Status::Unset),
            (None, Status::Ok)
        ]
    );
}

#[test]
fn every_kind_of_span_is_read_as_itself() {
    let kinds: Vec<_> = (0..=5).map(|kind| span(&json!({ "kind": kind }))).collect();

    let exported = Exported::parse(&line_of_spans(&kinds)).unwrap();

    let read: Vec<_> = exported.spans.iter().map(|span| span.kind).collect();
    assert_eq!(
        read,
        [
            SpanKind::Unspecified,
            SpanKind::Internal,
            SpanKind::Server,
            SpanKind::Client,
            SpanKind::Producer,
            SpanKind::Consumer,
        ]
    );
}

#[test]
fn a_span_that_ends_before_it_starts_lasted_no_time() {
    let exported = Exported::parse(&line_of_spans(&[span(&json!({
        "startTimeUnixNano": "2000000",
        "endTimeUnixNano": "1000000",
    }))]))
    .unwrap();

    assert_eq!(exported.spans[0].duration_ms(), 0);
}

#[test]
fn a_log_record_is_read_with_its_context_its_times_and_its_severity() {
    let exported = Exported::parse(&line_of_records(&[json!({
        "timeUnixNano": "1790000000255000000",
        "observedTimeUnixNano": "1790000000255000001",
        "severityNumber": 13,
        "severityText": "WARN",
        "eventName": "gen_ai.client.operation.exception",
        "body": { "stringValue": "run summary" },
        "attributes": [{ "key": "exception.type", "value": { "stringValue": "auth" } }],
        "flags": 1,
        "traceId": "abababababababababababababababab",
        "spanId": "cdcdcdcdcdcdcdcd",
    })]))
    .unwrap();

    assert_eq!(
        exported.records,
        [LogRecord {
            line: 1,
            resource: Attributes::new(),
            scope: Scope {
                name: "lablet".to_owned(),
                version: String::new(),
                schema_url: String::new(),
            },
            event_name: "gen_ai.client.operation.exception".to_owned(),
            severity_number: 13,
            severity_text: "WARN".to_owned(),
            time_unix_nano: 1_790_000_000_255_000_000,
            observed_time_unix_nano: 1_790_000_000_255_000_001,
            trace_id: "abababababababababababababababab".to_owned(),
            span_id: "cdcdcdcdcdcdcdcd".to_owned(),
            flags: 1,
            attributes: Attributes::from([("exception.type".to_owned(), json!("auth"))]),
            body: Some(json!("run summary")),
        }]
    );
}

#[test]
fn a_value_is_read_as_the_json_it_would_be_written_as() {
    let attribute = |key: &str, value: Value| json!({ "key": key, "value": value });
    let exported = Exported::parse(&line_of_records(&[json!({
        "attributes": [
            attribute("text", json!({ "stringValue": "chat" })),
            attribute("flag", json!({ "boolValue": false })),
            attribute("integer", json!({ "intValue": "-42" })),
            attribute("bare integer", json!({ "intValue": 7 })),
            attribute("float", json!({ "doubleValue": 2.0 })),
            attribute("bytes", json!({ "bytesValue": "AAH/" })),
            attribute("nothing", json!({})),
            attribute("list", json!({ "arrayValue": { "values": [
                { "stringValue": "end_turn" },
                { "intValue": "3" },
            ] } })),
            attribute("map", json!({ "kvlistValue": { "values": [
                attribute("role", json!({ "stringValue": "user" })),
                attribute("parts", json!({ "arrayValue": { "values": [] } })),
            ] } })),
        ],
    })]))
    .unwrap();

    assert_eq!(
        serde_json::to_value(&exported.records[0].attributes).unwrap(),
        json!({
            "text": "chat",
            "flag": false,
            "integer": -42,
            "bare integer": 7,
            "float": 2.0,
            "bytes": "0001ff",
            "nothing": null,
            "list": ["end_turn", 3],
            "map": { "role": "user", "parts": [] },
        })
    );
    assert_ne!(
        exported.records[0].attributes["float"],
        json!(2),
        "an integer and a float of one value differ, as their attributes' types do"
    );
}

#[test]
fn spans_are_found_by_their_operation_and_records_by_their_event() {
    let text = [
        line_of_spans(&[
            span(&json!({ "name": "chat scripted-1" })),
            span(&json!({ "name": "chatty" })),
            span(&json!({ "name": "chat" })),
            span(&json!({ "name": "execute_tool chat" })),
            span(&json!({ "name": "chat scripted-2" })),
        ]),
        line_of_records(&[
            json!({ "eventName": "lablet.run" }),
            json!({ "eventName": "lablet.running" }),
            json!({}),
        ]),
    ]
    .concat();

    let exported = Exported::parse(&text).unwrap();

    let chats: Vec<_> = exported
        .spans_of("chat")
        .into_iter()
        .map(|span| span.name.as_str())
        .collect();
    assert_eq!(chats, ["chat scripted-1", "chat scripted-2"]);
    assert_eq!(exported.spans_of("invoke_agent"), Vec::<&Span>::new());
    assert_eq!(exported.records_of("lablet.run").len(), 1);
    assert_eq!(exported.records_of("lablet").len(), 0);
}

// What's refused

#[test]
fn a_line_that_is_not_one_export_is_refused_and_named() {
    let good = line_of_spans(&[span(&json!({}))]);
    let both = "{\"resourceSpans\":[],\"resourceLogs\":[]}\n";
    for (text, reason) in [
        (
            format!("{good}\n{good}"),
            "line 2: it isn't JSON: EOF while parsing a value at line 1 column 0",
        ),
        (
            format!("{good}{{\"resourceSpans\n"),
            "line 2: it isn't JSON: EOF while parsing a string at line 1 column 15",
        ),
        (format!("{good}[]\n"), "line 2: it isn't a JSON object"),
        (
            format!("{good}{good}{{}}\n"),
            "line 3: its keys are [], and an export has one, `resourceSpans` or `resourceLogs`",
        ),
        (
            both.to_owned(),
            "line 1: its keys are [\"resourceLogs\", \"resourceSpans\"], and an export has one, `resourceSpans` or `resourceLogs`",
        ),
        (
            "{\"resourceMetrics\":[]}\n".to_owned(),
            "line 1: its keys are [\"resourceMetrics\"], and an export has one, `resourceSpans` or `resourceLogs`",
        ),
        (
            "{\"resourceSpans\":[],\"lablet\":1}\n".to_owned(),
            "line 1: its keys are [\"lablet\", \"resourceSpans\"], and an export has one, `resourceSpans` or `resourceLogs`",
        ),
    ] {
        assert_eq!(fault(&text), reason, "{text:?}");
    }
}

#[test]
fn a_line_that_does_not_read_as_the_request_its_key_names_is_refused() {
    for (text, reason) in [
        (
            "{\"resourceSpans\":{}}\n",
            "line 1: it isn't an export of spans: invalid type: map, expected a sequence",
        ),
        (
            "{\"resourceLogs\":7}\n",
            "line 1: it isn't an export of log records: invalid type: integer `7`, expected a sequence",
        ),
    ] {
        assert_eq!(fault(text), reason);
    }
    assert!(
        fault(&line_of_spans(&[span(&json!({ "traceId": "not hex" }))]))
            .starts_with("line 1: it isn't an export of spans: "),
    );
}

#[test]
fn lines_that_were_cut_short_are_refused() {
    let good = line_of_spans(&[span(&json!({}))]);
    let cut = good.trim_end();

    assert_eq!(
        fault(&format!("{good}{cut}")),
        "line 2: it doesn't end in a newline, so the export was cut short"
    );
    assert_eq!(
        fault("{\"resourceSp"),
        "line 1: it doesn't end in a newline, so the export was cut short"
    );
}

#[test]
fn a_key_that_is_there_twice_is_refused_wherever_it_is() {
    let twice = json!([
        { "key": "lablet.turn", "value": { "intValue": "1" } },
        { "key": "lablet.attempt", "value": { "intValue": "1" } },
        { "key": "lablet.turn", "value": { "intValue": "2" } },
    ]);
    let nested = json!([{
        "key": "gen_ai.input.messages",
        "value": { "arrayValue": { "values": [{ "kvlistValue": { "values": twice } }] } },
    }]);
    let of_the_resource = json!({
        "resourceSpans": [{ "resource": { "attributes": twice }, "scopeSpans": [] }],
    });

    for text in [
        line_of_spans(&[span(&json!({ "attributes": twice }))]),
        line_of_spans(&[span(&json!({ "events": [{ "attributes": twice }] }))]),
        line_of_records(&[json!({ "attributes": twice })]),
        line_of_records(&[json!({ "attributes": nested })]),
        line_of_records(&[json!({ "body": { "kvlistValue": { "values": twice } } })]),
        format!("{of_the_resource}\n"),
    ] {
        assert_eq!(
            fault(&text),
            "line 1: the key `lablet.turn` is there twice",
            "{text}"
        );
    }
}

#[test]
fn a_kind_or_a_status_no_span_has_is_refused() {
    assert_eq!(
        fault(&line_of_spans(&[span(&json!({ "kind": 6 }))])),
        "line 1: 6 isn't a kind of span"
    );
    assert_eq!(
        fault(&line_of_spans(&[span(&json!({ "status": { "code": 3 } }))])),
        "line 1: 3 isn't a status of a span"
    );
}

#[test]
fn what_the_reader_does_not_carry_is_refused_and_named() {
    let link =
        json!({ "traceId": "abababababababababababababababab", "spanId": "0101010101010101" });
    let attribute = json!([{ "key": "lablet.turn", "value": { "intValue": "1" } }]);
    let mut of_the_scope: Value =
        serde_json::from_str(line_of_spans(&[span(&json!({}))]).trim_end()).unwrap();
    of_the_scope["resourceSpans"][0]["scopeSpans"][0]["scope"]["attributes"] = attribute;

    for (text, what) in [
        (
            line_of_spans(&[span(&json!({ "links": [link] }))]),
            "the span `chat scripted-1` has links",
        ),
        (
            line_of_spans(&[span(&json!({ "traceState": "lablet=1" }))]),
            "the span `chat scripted-1` has a trace state",
        ),
        (
            line_of_spans(&[span(&json!({ "droppedAttributesCount": 1 }))]),
            "the span `chat scripted-1` has a count of dropped attributes",
        ),
        (
            line_of_spans(&[span(&json!({ "droppedEventsCount": 1 }))]),
            "the span `chat scripted-1` has a count of dropped events",
        ),
        (
            line_of_spans(&[span(&json!({ "droppedLinksCount": 1 }))]),
            "the span `chat scripted-1` has a count of dropped links",
        ),
        (
            line_of_spans(&[span(
                &json!({ "events": [{ "name": "lablet.retry", "droppedAttributesCount": 1 }] }),
            )]),
            "the event `lablet.retry` has a count of dropped attributes",
        ),
        (
            line_of_records(&[json!({ "eventName": "lablet.run", "droppedAttributesCount": 1 })]),
            "the record `lablet.run` has a count of dropped attributes",
        ),
        (
            format!("{of_the_scope}\n"),
            "the scope `lablet` has attributes",
        ),
    ] {
        assert_eq!(
            fault(&text),
            format!("line 1: {what}, which the reader doesn't carry"),
            "{text}"
        );
    }
}

#[test]
fn a_value_that_refers_to_a_table_of_strings_is_refused() {
    let text = line_of_records(&[json!({
        "attributes": [{ "key": "text", "value": { "stringValueStrindex": 3 } }],
    })]);

    // The serde form of the message doesn't know the key, so the value
    // reads as none at all.
    assert_eq!(
        Exported::parse(&text).unwrap().records[0].attributes["text"],
        Value::Null
    );
    assert_eq!(
        json(AnyValue {
            value: Some(any_value::Value::StringValueStrindex(3))
        }),
        Err("a value refers to a table of strings, which an export has none of".to_owned())
    );
}

// Reading a file

#[test]
fn a_file_is_read_as_its_text_is() {
    let scratch = lablet_test_support::Scratch::new("conformance-read");
    let path = scratch.at("a-file-is-read.otlp.jsonl");
    let text = line_of_spans(&[span(&json!({}))]);
    std::fs::write(&path, &text).unwrap();

    let read = Exported::read(&path);

    assert_eq!(read, Exported::parse(&text));
    assert_eq!(read.unwrap().spans.len(), 1);
}

#[test]
fn a_file_that_cannot_be_read_is_an_error_that_names_it() {
    let scratch = lablet_test_support::Scratch::new("conformance-unread");
    let path = scratch.at("never-written.otlp.jsonl");

    let refused = Exported::read(&path).unwrap_err();

    let ReadError::Unreadable {
        path: named,
        reason,
    } = &refused
    else {
        panic!("{refused:?} isn't a file that couldn't be read");
    };
    assert_eq!(named, &path);
    assert_eq!(
        refused.to_string(),
        format!("{} couldn't be read: {reason}", path.display())
    );
}
