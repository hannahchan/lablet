use serde_json::json;

use super::*;

fn docs_server() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
}

// The literal spellings are the members of `lablet.tool.source` in the
// telemetry registry, which this crate can't depend on.
#[test]
fn a_tool_source_prints_the_telemetry_spelling_of_its_variant() {
    assert_eq!(ToolSource::Builtin.to_string(), "builtin");
    assert_eq!(docs_server().to_string(), "mcp");
}

#[test]
fn a_tool_source_serialises_under_the_spelling_it_prints() {
    assert_eq!(
        serde_json::to_value(ToolSource::Builtin).unwrap(),
        json!("builtin")
    );
    assert_eq!(
        serde_json::to_value(docs_server()).unwrap(),
        json!({ "mcp": { "server": "docs" } })
    );
    assert_eq!(
        serde_json::from_value::<ToolSource>(json!({ "mcp": { "server": "docs" } })).unwrap(),
        docs_server()
    );
    assert_eq!(
        serde_json::from_value::<ToolSource>(json!("builtin")).unwrap(),
        ToolSource::Builtin
    );
}

#[test]
fn a_network_transport_prints_what_it_serialises_as() {
    for (transport, spelling) in [
        (NetworkTransport::Pipe, "pipe"),
        (NetworkTransport::Tcp, "tcp"),
    ] {
        assert_eq!(transport.to_string(), spelling);
        assert_eq!(serde_json::to_value(transport).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<NetworkTransport>(json!(spelling)).unwrap(),
            transport
        );
    }
}

#[test]
fn a_tool_spec_has_one_json_form() {
    let spec = ToolSpec {
        name: ToolName::new("search").unwrap(),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "type": "object" }),
        source: docs_server(),
    };
    let expected = json!({
        "name": "search",
        "description": "Search the docs.",
        "input_schema": { "type": "object" },
        "source": { "mcp": { "server": "docs" } },
    });

    assert_eq!(serde_json::to_value(&spec).unwrap(), expected);
    assert_eq!(serde_json::from_value::<ToolSpec>(expected).unwrap(), spec);
}

#[test]
fn mcp_call_meta_and_trace_context_have_one_json_form() {
    let meta = McpCallMeta {
        method: "tools/call".to_owned(),
        session_id: Some("s-1".to_owned()),
        protocol_version: Some("2025-06-18".to_owned()),
        jsonrpc_request_id: Some("4".to_owned()),
        rpc_status_code: None,
        transport: NetworkTransport::Tcp,
    };
    let context = TraceContext {
        traceparent: "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_owned(),
        tracestate: None,
    };

    let meta_json = json!({
        "method": "tools/call",
        "session_id": "s-1",
        "protocol_version": "2025-06-18",
        "jsonrpc_request_id": "4",
        "rpc_status_code": null,
        "transport": "tcp",
    });
    assert_eq!(serde_json::to_value(&meta).unwrap(), meta_json);
    assert_eq!(
        serde_json::from_value::<McpCallMeta>(meta_json).unwrap(),
        meta
    );
    assert_eq!(
        serde_json::to_value(&context).unwrap(),
        json!({
            "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            "tracestate": null,
        })
    );
}
