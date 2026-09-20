//! What a run offers, and what it refuses to be built from.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{CompletionMode, ToolCallId, ToolName, ToolSource, ToolSpec};

use super::fakes::{Answers, FakeClock, FakeTools};
use crate::{ToolCall, ToolErrorKind, ToolExecutor, ToolFilter, ToolSet, ToolSetError};

fn name(value: &str) -> ToolName {
    ToolName::new(value).expect("a test's tool name is valid")
}

fn spec(value: &str, source: ToolSource) -> ToolSpec {
    ToolSpec {
        name: name(value),
        description: format!("The {value} tool."),
        input_schema: serde_json::json!({ "type": "object" }),
        source,
    }
}

fn executor(specs: Vec<ToolSpec>) -> Arc<dyn ToolExecutor> {
    Arc::new(FakeTools::new(Arc::new(FakeClock::new()), specs))
}

fn mcp(server: &str) -> ToolSource {
    ToolSource::Mcp {
        server: server.to_owned(),
    }
}

async fn built(
    executors: Vec<Arc<dyn ToolExecutor>>,
    filter: ToolFilter,
) -> Result<ToolSet, ToolSetError> {
    ToolSet::build(executors, &filter, CompletionMode::Natural, None).await
}

fn offered(set: &ToolSet) -> Vec<&ToolName> {
    set.specs().iter().map(|spec| &spec.name).collect()
}

#[tokio::test]
async fn every_tool_every_executor_serves_is_offered_when_nothing_is_filtered() {
    let set = built(
        vec![
            executor(vec![spec("bash", ToolSource::Builtin)]),
            executor(vec![spec("search", mcp("docs"))]),
        ],
        ToolFilter::default(),
    )
    .await
    .expect("distinct names");

    assert_eq!(offered(&set), [&name("bash"), &name("search")]);
    assert_eq!(set.source(&name("bash")), Some(&ToolSource::Builtin));
    assert_eq!(set.source(&name("search")), Some(&mcp("docs")));
}

#[tokio::test]
async fn an_allow_list_offers_only_what_it_names() {
    let set = built(
        vec![executor(vec![
            spec("bash", ToolSource::Builtin),
            spec("read_file", ToolSource::Builtin),
        ])],
        ToolFilter {
            allow: vec![name("bash")],
            deny: Vec::new(),
        },
    )
    .await
    .expect("distinct names");

    assert_eq!(offered(&set), [&name("bash")]);
    assert_eq!(
        set.source(&name("read_file")),
        None,
        "a tool that wasn't offered can't be reported as one that ran"
    );
}

#[tokio::test]
async fn a_deny_list_removes_what_it_names_even_when_the_allow_list_has_it() {
    let set = built(
        vec![executor(vec![
            spec("bash", ToolSource::Builtin),
            spec("read_file", ToolSource::Builtin),
        ])],
        ToolFilter {
            allow: vec![name("bash"), name("read_file")],
            deny: vec![name("bash")],
        },
    )
    .await
    .expect("distinct names");

    assert_eq!(
        offered(&set),
        [&name("read_file")],
        "denying is the safer reading of a contradiction"
    );
}

#[tokio::test]
async fn two_executors_serving_one_name_is_refused() {
    let refused = built(
        vec![
            executor(vec![spec("search", mcp("docs"))]),
            executor(vec![spec("search", mcp("code"))]),
        ],
        ToolFilter::default(),
    )
    .await
    .expect_err("one name, two servers");

    assert_eq!(
        refused,
        ToolSetError::DuplicateName {
            name: name("search")
        }
    );
}

/// The name is claimed before the filter is asked, so a denied tool doesn't
/// quietly let a second executor take its name.
#[tokio::test]
async fn a_denied_name_is_still_a_duplicate() {
    let refused = built(
        vec![
            executor(vec![spec("search", mcp("docs"))]),
            executor(vec![spec("search", mcp("code"))]),
        ],
        ToolFilter {
            allow: Vec::new(),
            deny: vec![name("search")],
        },
    )
    .await
    .expect_err("a denied name is claimed all the same");

    assert_eq!(
        refused,
        ToolSetError::DuplicateName {
            name: name("search")
        }
    );
}

#[tokio::test]
async fn an_executor_that_cannot_list_its_tools_refuses_the_run() {
    let refused = ToolSet::build(
        vec![Arc::new(
            FakeTools::new(Arc::new(FakeClock::new()), Vec::new())
                .cannot_list(ToolErrorKind::Failed),
        )],
        &ToolFilter::default(),
        CompletionMode::Natural,
        None,
    )
    .await
    .expect_err("the executor couldn't say what it offers");

    assert!(matches!(refused, ToolSetError::Specs(_)));
    assert!(refused.to_string().contains("couldn't list its tools"));
}

#[tokio::test]
async fn task_complete_is_offered_in_explicit_mode_only_and_never_routed() {
    let natural = built(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        ToolFilter::default(),
    )
    .await
    .expect("distinct names");
    let explicit = ToolSet::build(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        &ToolFilter::default(),
        CompletionMode::Explicit,
        None,
    )
    .await
    .expect("distinct names");

    assert_eq!(offered(&natural), [&name("bash")]);
    assert!(!natural.is_task_complete(&ToolName::task_complete()));

    assert_eq!(
        offered(&explicit),
        [&name("bash"), &ToolName::task_complete()],
        "the completion tool is offered last"
    );
    assert!(explicit.is_task_complete(&ToolName::task_complete()));
    assert_eq!(
        explicit.source(&ToolName::task_complete()),
        None,
        "it's intercepted rather than routed, so it has no executor to name"
    );
}

#[tokio::test]
async fn the_completion_tools_schema_is_the_runs_when_it_has_one() {
    let schema = serde_json::json!({ "type": "object", "required": ["passed"] });
    let set = ToolSet::build(
        Vec::new(),
        &ToolFilter::default(),
        CompletionMode::Explicit,
        Some(schema.clone()),
    )
    .await
    .expect("nothing to conflict");

    assert_eq!(set.specs()[0].input_schema, schema);

    let free = ToolSet::build(
        Vec::new(),
        &ToolFilter::default(),
        CompletionMode::Explicit,
        None,
    )
    .await
    .expect("nothing to conflict");
    assert_eq!(
        free.specs()[0].input_schema,
        serde_json::json!({ "type": "object" })
    );
}

/// The filter doesn't reach the completion tool: a run that completes
/// explicitly has no way to finish without it.
#[tokio::test]
async fn the_completion_tool_survives_an_allow_list_that_does_not_name_it() {
    let set = ToolSet::build(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        &ToolFilter {
            allow: vec![name("bash")],
            deny: Vec::new(),
        },
        CompletionMode::Explicit,
        None,
    )
    .await
    .expect("distinct names");

    assert_eq!(offered(&set), [&name("bash"), &ToolName::task_complete()]);
}

#[tokio::test]
async fn a_call_is_routed_to_whichever_executor_serves_its_name() {
    let clock = Arc::new(FakeClock::new());
    let set = ToolSet::build(
        vec![
            Arc::new(
                FakeTools::new(Arc::clone(&clock), vec![spec("bash", ToolSource::Builtin)])
                    .answers("bash", Answers::Text("from the first".to_owned())),
            ),
            Arc::new(
                FakeTools::new(Arc::clone(&clock), vec![spec("search", mcp("docs"))])
                    .answers("search", Answers::Text("from the second".to_owned())),
            ),
        ],
        &ToolFilter::default(),
        CompletionMode::Natural,
        None,
    )
    .await
    .expect("distinct names");

    let output = set
        .execute(call("search"))
        .await
        .expect("the second executor answers");

    assert_eq!(
        output.content,
        vec![lablet_model::ToolResultContent::Text(
            "from the second".to_owned()
        )]
    );
}

#[tokio::test]
async fn a_call_to_a_name_this_run_does_not_offer_is_unknown() {
    let set = built(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        ToolFilter {
            allow: Vec::new(),
            deny: vec![name("bash")],
        },
    )
    .await
    .expect("distinct names");

    let refused = set.execute(call("bash")).await.expect_err("it was denied");

    assert_eq!(refused.kind, ToolErrorKind::Unknown);
    assert!(refused.message.contains("bash"));
}

fn call(name_: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("call_0").expect("a valid call id"),
        name: name(name_),
        input: serde_json::json!({}),
        deadline: Duration::from_secs(30),
        trace_context: None,
    }
}

#[tokio::test]
async fn a_tool_set_prints_what_the_run_offers() {
    let set = built(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        ToolFilter::default(),
    )
    .await
    .expect("distinct names");

    let printed = format!("{set:?}");
    assert!(printed.starts_with("ToolSet {"), "{printed}");
    assert!(printed.contains("bash"), "{printed}");
}

#[test]
fn a_transport_prints_the_value_its_attribute_takes() {
    assert_eq!(crate::NetworkTransport::Pipe.as_str(), "pipe");
    assert_eq!(crate::NetworkTransport::Tcp.as_str(), "tcp");
}
