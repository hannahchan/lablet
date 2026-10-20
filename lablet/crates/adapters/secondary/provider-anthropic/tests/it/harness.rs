//! What the tests share: a wiremock server standing in for the API, a
//! target and a provider at it, requests and responses in the API's shapes,
//! an executor that answers every call, the loop around the provider, and
//! the spans the loop opened.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use lablet_model::{
    FinishedRun, KeptOutput, Message, ProviderResponse, Thinking, ToolConcurrency, ToolName,
    ToolSource, ToolSpec, UserContent,
};
use lablet_provider_anthropic::{AnthropicProvider, Key, Target};
use lablet_run::telemetry::generated::{LabletChat, key};
use lablet_run::{
    ModelProvider, ProviderError, ProviderRequest, ToolCall, ToolError, ToolExecutor, ToolOutput,
};
use lablet_test_support::{PROMPT, RunBuilder, SYSTEM, context, prompts};
use opentelemetry::global::BoxedTracer;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::trace::noop::NoopTextMapPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A key of the shape the API hands out, made up here; long enough for the
/// loop's cut to hold it.
pub const KEY: &str = "sk-ant-api03-made-up-for-the-tests-0123456789abcdef";

/// The model every request names.
pub const MODEL: &str = "claude-sonnet-5-5";

/// The model the responses say answered.
pub const ANSWERED_BY: &str = "claude-sonnet-5-5-20260301";

/// How long an attempt has where a test doesn't ask: long enough that no
/// test reaches it.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// A target at `server`, reached with [`KEY`], naming [`MODEL`].
pub fn target(server: &MockServer, cache: bool) -> Target {
    Target::new(MODEL, Key(KEY), Some(&server.uri()), cache).unwrap()
}

/// A provider at `server` that sends no markers, injects through no
/// propagator and captures no content.
pub fn provider(server: &MockServer) -> AnthropicProvider {
    with(target(server, false), NoopTextMapPropagator::new(), false)
}

/// A provider at `target` that injects through `propagator`.
pub fn with(
    target: Target,
    propagator: impl TextMapPropagator + Send + Sync + 'static,
    capture_content: bool,
) -> AnthropicProvider {
    AnthropicProvider::new(target, Arc::new(propagator), capture_content)
}

/// The task prompt, as a user message's input.
pub fn prompt() -> Vec<UserContent> {
    vec![UserContent::Text(PROMPT.to_owned())]
}

/// One attempt over `messages`, offering `tools`, with the loop's defaults
/// and [`DEADLINE`].
pub fn request<'a>(messages: &'a [Message<'a>], tools: &'a [ToolSpec]) -> ProviderRequest<'a> {
    ProviderRequest {
        system: SYSTEM,
        messages,
        tools,
        max_tokens: 1_024,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        thinking_display: None,
        effort: None,
        seed: None,
        cache_key: None,
        deadline: DEADLINE,
    }
}

/// One attempt of the task prompt alone against `provider`.
pub async fn complete(provider: &AnthropicProvider) -> Result<ProviderResponse, ProviderError> {
    let input = prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    provider.complete(request(&messages, &[])).await
}

/// A message of `content` that stopped for `stop_reason`, with the usage
/// of a small call.
pub fn message(content: Value, stop_reason: &str) -> Value {
    let mut message = json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": ANSWERED_BY,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": { "input_tokens": 12, "output_tokens": 5 }
    });
    message["content"] = content;
    message
}

/// A message of one text block that ended the turn.
pub fn text(text: &str) -> Value {
    message(json!([{ "type": "text", "text": text }]), "end_turn")
}

/// A 200 carrying `body`.
pub fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// A 200 of a message whose `content` array is sent as the text is written,
/// so a `tool_use` input's keys arrive in the model's order, where `ok` would
/// sort them before serving.
pub fn ok_as_written(content: &str, stop_reason: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(
        format!(
            concat!(
                r#"{{"id":"msg_01","type":"message","role":"assistant","model":"{model}","#,
                r#""stop_reason":"{stop_reason}","stop_sequence":null,"#,
                r#""usage":{{"input_tokens":12,"output_tokens":5}},"content":{content}}}"#
            ),
            model = ANSWERED_BY,
            stop_reason = stop_reason,
            content = content,
        ),
        "application/json",
    )
}

/// A failure in the API's envelope, with a request id in the body.
pub fn envelope(status: u16, kind: &str, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "type": "error",
        "error": { "type": kind, "message": message },
        "request_id": "req_011CSHoEeqs5C35K2UUqR7Fy"
    }))
}

/// Answers every `POST /v1/messages` at `server` with `template`.
pub async fn answer(server: &MockServer, template: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Answers each `POST /v1/messages` at `server` with the next of
/// `templates`, and every one past the last with the last.
pub async fn answer_in_turn(server: &MockServer, templates: Vec<ResponseTemplate>) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Sequence {
            templates,
            next: AtomicUsize::new(0),
        })
        .mount(server)
        .await;
}

struct Sequence {
    templates: Vec<ResponseTemplate>,
    next: AtomicUsize,
}

impl Respond for Sequence {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let next = self.next.fetch_add(1, Ordering::Relaxed);
        self.templates[next.min(self.templates.len() - 1)].clone()
    }
}

/// Every request `server` received, in order.
pub async fn received(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

/// The body of `request`, read as JSON.
pub fn body(request: &Request) -> Value {
    request.body_json().unwrap()
}

/// The header `name` of `request`, when it has one.
pub fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .map(|value| value.to_str().unwrap())
}

/// An executor that offers tools by name and answers every call with `ok`.
pub struct Tools(Vec<ToolSpec>);

impl Tools {
    /// An executor offering the tools `names` name, each taking a `path`.
    pub fn offering(names: &[&str]) -> Arc<dyn ToolExecutor> {
        Arc::new(Self(names.iter().map(|name| spec(name)).collect()))
    }
}

/// The spec of a tool named `name` that takes a `path`.
pub fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name).unwrap(),
        description: format!("The {name} tool."),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        source: ToolSource::Builtin,
        concurrency: ToolConcurrency::Shared,
    }
}

#[async_trait::async_trait]
impl ToolExecutor for Tools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        Ok(self.0.clone())
    }

    async fn execute(&self, _: ToolCall) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            output: KeptOutput::whole("ok"),
            is_error: false,
            mcp: None,
        })
    }
}

/// The spans the loop opened, read from the SDK's in-memory exporter.
pub struct Recorder {
    spans: InMemorySpanExporter,
    /// Held because an in-memory exporter forgets everything when its
    /// provider shuts down.
    _provider: SdkTracerProvider,
}

impl Recorder {
    /// A recorder and the tracer the loop opens its spans through.
    pub fn new() -> (Self, BoxedTracer) {
        let spans = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .build();
        let tracer = BoxedTracer::new(Box::new(provider.tracer("test")));
        (
            Self {
                spans,
                _provider: provider,
            },
            tracer,
        )
    }

    /// The chat spans, one for each attempt of a provider call, in the
    /// order the attempts started.
    pub fn chats(&self) -> Vec<SpanData> {
        let mut chats: Vec<SpanData> = self
            .spans
            .get_finished_spans()
            .unwrap()
            .into_iter()
            .filter(|span| {
                span.attributes.iter().any(|attribute| {
                    attribute.key.as_str() == key::GEN_AI_OPERATION_NAME
                        && attribute.value.as_str() == LabletChat::GEN_AI_OPERATION_NAME
                })
            })
            .collect();
        chats.sort_by_key(|span| span.start_time);
        chats
    }
}

/// The loop around `provider`, offering `tools`, which retries a failed
/// attempt up to `max_retries` times.
pub fn looped(
    provider: AnthropicProvider,
    tools: Vec<Arc<dyn ToolExecutor>>,
    max_retries: u32,
) -> RunBuilder {
    RunBuilder::new(Arc::new(provider))
        .tools(tools)
        .max_retries(max_retries)
}

/// One run of the loop `builder` makes.
pub async fn run(builder: RunBuilder) -> FinishedRun {
    builder
        .build()
        .await
        .run(context("01K8ANTHROPICRUN0000000000"), prompts())
        .await
}
