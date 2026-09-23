//! What a run offers, and what it refuses to be built from.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    CompletionMode, ToolCallId, ToolConcurrency, ToolInput, ToolName, ToolSource, ToolSpec, ToolUse,
};

use super::fakes::{Answers, FakeClock, FakeTools};
use crate::{FilterList, ToolCall, ToolErrorKind, ToolExecutor, ToolFilter, ToolSet, ToolSetError};

fn name(value: &str) -> ToolName {
    ToolName::new(value).expect("a test's tool name is valid")
}

fn spec(value: &str, source: ToolSource) -> ToolSpec {
    ToolSpec {
        name: name(value),
        description: format!("The {value} tool."),
        input_schema: serde_json::json!({ "type": "object" }),
        source,
        concurrency: ToolConcurrency::Exclusive,
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

    assert_eq!(
        offered(&explicit),
        [&name("bash"), &ToolName::task_complete()],
        "the completion tool is offered last"
    );
    assert_eq!(
        explicit.source(&ToolName::task_complete()),
        Some(&ToolSource::Builtin),
        "it's offered, so a call to it with bad arguments names a real tool"
    );
    assert_eq!(
        explicit.concurrency(&ToolUse {
            id: ToolCallId::new("call_0").expect("a valid call id"),
            name: ToolName::task_complete(),
            input: ToolInput::Json(serde_json::json!({})),
        }),
        ToolConcurrency::Shared,
        "it's never executed, so it can't change what another call sees"
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

/// The loop resolves a name before it reaches an executor, so its own routing
/// never returns this kind. An executor may, and the port says what it means:
/// no tool ran, so the call has no ending.
#[test]
fn only_the_two_kinds_that_mean_a_tool_ran_have_an_ending() {
    use lablet_model::ToolCallEnd;

    assert_eq!(ToolErrorKind::Unknown.ended(), None);
    assert_eq!(ToolErrorKind::Timeout.ended(), Some(ToolCallEnd::Timeout));
    assert_eq!(ToolErrorKind::Failed.ended(), Some(ToolCallEnd::Failed));
}

/// The loop intercepts this name rather than routing it, so an executor that
/// also serves it would be offered twice — which every provider rejects — and
/// its own tool could never run.
#[tokio::test]
async fn an_executor_that_serves_task_complete_collides_with_the_built_in_one() {
    let refused = ToolSet::build(
        vec![executor(vec![spec(
            CompletionMode::TASK_COMPLETE,
            ToolSource::Builtin,
        )])],
        &ToolFilter::default(),
        CompletionMode::Explicit,
        None,
    )
    .await
    .expect_err("the run already offers that name");

    assert_eq!(
        refused,
        ToolSetError::DuplicateName {
            name: ToolName::task_complete()
        }
    );
}

/// Nothing registers the name in natural mode, so it's an ordinary tool.
#[tokio::test]
async fn natural_mode_lets_an_executor_serve_a_tool_called_task_complete() {
    let set = built(
        vec![executor(vec![spec(
            CompletionMode::TASK_COMPLETE,
            ToolSource::Builtin,
        )])],
        ToolFilter::default(),
    )
    .await
    .expect("nothing else claims the name");

    assert_eq!(offered(&set), [&ToolName::task_complete()]);
    assert_eq!(
        set.source(&ToolName::task_complete()),
        Some(&ToolSource::Builtin)
    );
}

/// The set is the run's one copy of the mode, so everything that needs it
/// reads it back from the thing whose shape it decided.
#[tokio::test]
async fn a_tool_set_reports_the_mode_it_was_built_for() {
    let natural = built(Vec::new(), ToolFilter::default())
        .await
        .expect("nothing to conflict");
    let explicit = ToolSet::build(
        Vec::new(),
        &ToolFilter::default(),
        CompletionMode::Explicit,
        None,
    )
    .await
    .expect("nothing to conflict");

    assert_eq!(natural.completion(), CompletionMode::Natural);
    assert_eq!(explicit.completion(), CompletionMode::Explicit);
}

#[tokio::test]
async fn a_filter_name_that_no_executor_serves_is_refused() {
    for (filter, list) in [
        (
            ToolFilter {
                allow: vec![name("bash"), name("raed_file")],
                deny: Vec::new(),
            },
            FilterList::Allow,
        ),
        (
            ToolFilter {
                allow: Vec::new(),
                deny: vec![name("raed_file")],
            },
            FilterList::Deny,
        ),
    ] {
        let refused = built(
            vec![executor(vec![
                spec("bash", ToolSource::Builtin),
                spec("read_file", ToolSource::Builtin),
            ])],
            filter,
        )
        .await
        .unwrap_err();

        assert_eq!(
            refused,
            ToolSetError::UnknownFilterName {
                name: name("raed_file"),
                list,
            }
        );
    }
}

#[tokio::test]
async fn a_misspelt_deny_entry_is_named_in_words() {
    let refused = built(
        vec![executor(vec![spec("read_file", ToolSource::Builtin)])],
        ToolFilter {
            allow: Vec::new(),
            deny: vec![name("raed_file")],
        },
    )
    .await
    .unwrap_err();

    assert_eq!(
        refused.to_string(),
        "the deny list names raed_file, which no tool is called"
    );
    assert_eq!(FilterList::Allow.to_string(), "allow");
}

/// The filter doesn't apply to `task_complete` in explicit mode, so naming it
/// can only be a mistake. In natural mode an executor may serve a tool by
/// that name, and filtering it is an ordinary filter.
#[tokio::test]
async fn task_complete_can_be_filtered_only_when_an_executor_serves_it() {
    let deny = ToolFilter {
        allow: Vec::new(),
        deny: vec![ToolName::task_complete()],
    };

    let explicit = ToolSet::build(
        vec![executor(vec![spec("bash", ToolSource::Builtin)])],
        &deny,
        CompletionMode::Explicit,
        None,
    )
    .await;
    let natural = built(
        vec![executor(vec![spec(
            CompletionMode::TASK_COMPLETE,
            ToolSource::Builtin,
        )])],
        deny,
    )
    .await
    .expect("an executor serves it");

    assert_eq!(
        explicit.unwrap_err(),
        ToolSetError::UnknownFilterName {
            name: ToolName::task_complete(),
            list: FilterList::Deny,
        }
    );
    assert!(offered(&natural).is_empty());
}

#[tokio::test]
async fn a_call_runs_beside_others_only_when_its_tool_says_it_may() {
    let set = built(
        vec![executor(vec![
            ToolSpec {
                concurrency: ToolConcurrency::Shared,
                ..spec("read_file", ToolSource::Builtin)
            },
            spec("bash", ToolSource::Builtin),
        ])],
        ToolFilter::default(),
    )
    .await
    .expect("distinct names");

    let call = |tool: &str, input: ToolInput| ToolUse {
        id: ToolCallId::new("call_0").expect("a valid call id"),
        name: name(tool),
        input,
    };
    let parsed = || ToolInput::Json(serde_json::json!({}));

    assert_eq!(
        set.concurrency(&call("read_file", parsed())),
        ToolConcurrency::Shared
    );
    assert_eq!(
        set.concurrency(&call("bash", parsed())),
        ToolConcurrency::Exclusive
    );
    assert_eq!(
        set.concurrency(&call("rm_rf", parsed())),
        ToolConcurrency::Shared,
        "a name no tool has is answered without an executor, so it changes nothing"
    );
    assert_eq!(
        set.concurrency(&call("bash", ToolInput::Unparsed("{".to_owned()))),
        ToolConcurrency::Shared,
        "nor are arguments that didn't parse"
    );
}
