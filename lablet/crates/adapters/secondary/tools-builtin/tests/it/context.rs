//! The context a command starts in: its tool span's, injected into its
//! environment through the propagator the executor was built with, as the
//! command starts.

use std::sync::Arc;

use lablet_run::ToolExecutor as _;
use lablet_tools_builtin::BuiltinTools;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{
    Extractor, Injector, TextMapCompositePropagator, TextMapPropagator,
};
use opentelemetry::trace::{
    FutureExt as _, SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use serde_json::json;

use crate::harness::{Root, call, text};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";

/// The context of a sampled span of [`TRACE_ID`] whose id is `span`.
fn under(span: &str) -> Context {
    Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_hex(TRACE_ID).unwrap(),
        SpanId::from_hex(span).unwrap(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    ))
}

/// An executor of `bash` alone that injects through `propagator`.
fn tools(
    scratch: &Root,
    propagator: impl TextMapPropagator + Send + Sync + 'static,
) -> BuiltinTools {
    BuiltinTools::new(scratch.settings(), Arc::new(propagator)).unwrap()
}

/// What `command` printed, run in `context`.
async fn printed(tools: &BuiltinTools, command: &str, context: Context) -> String {
    let output = tools
        .execute(call("bash", json!({ "command": command })))
        .with_context(context)
        .await
        .unwrap();
    text(output.output)
}

/// What the command prints of the context variables it may be given.
const PRINTS: &str = "printf '%s|%s|%s|%s' \"${TRACEPARENT-unset}\" \"${TRACESTATE-unset}\" \
                      \"${BAGGAGE-unset}\" \"${B3-unset}\"";

#[tokio::test]
async fn a_command_is_given_the_context_of_its_tool_span() {
    let scratch = Root::new("context-span");
    let tools = tools(&scratch, TraceContextPropagator::new());

    let text = printed(&tools, PRINTS, under("b7ad6b7169203331")).await;

    assert_eq!(
        text,
        format!("00-{TRACE_ID}-b7ad6b7169203331-01|unset|unset|unset\nexit code: 0"),
        "an empty trace state is no TRACESTATE"
    );
}

#[tokio::test]
async fn a_command_s_baggage_is_the_context_s() {
    let scratch = Root::new("context-baggage");
    let tools = tools(
        &scratch,
        TextMapCompositePropagator::new(vec![
            Box::new(TraceContextPropagator::new()),
            Box::new(BaggagePropagator::new()),
        ]),
    );

    let context = under("b7ad6b7169203331").with_baggage([KeyValue::new("tenant", "acme")]);
    let text = printed(&tools, PRINTS, context).await;

    assert_eq!(
        text,
        format!("00-{TRACE_ID}-b7ad6b7169203331-01|unset|tenant=acme|unset\nexit code: 0")
    );
}

/// A propagator in B3's single-header form: `b3` is the trace id, the span
/// id and whether it's sampled.
#[derive(Debug)]
struct SingleHeader(Vec<String>);

impl TextMapPropagator for SingleHeader {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        let span = cx.span();
        let context = span.span_context();
        injector.set(
            "b3",
            format!("{}-{}-1", context.trace_id(), context.span_id()),
        );
    }

    fn extract_with_context(&self, cx: &Context, _: &dyn Extractor) -> Context {
        cx.clone()
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.0)
    }
}

#[tokio::test]
async fn a_command_is_injected_through_the_propagator_the_tools_were_built_with() {
    let scratch = Root::new("context-propagator");
    let tools = tools(&scratch, SingleHeader(vec!["b3".to_owned()]));

    let text = printed(&tools, PRINTS, under("b7ad6b7169203331")).await;

    assert_eq!(
        text,
        format!("unset|unset|unset|{TRACE_ID}-b7ad6b7169203331-1\nexit code: 0")
    );
}

/// The call's future is made while one span is current and awaited under
/// another, as the loop makes it and then runs it under its tool span: the
/// command is under the span current as it starts.
#[tokio::test]
async fn the_context_is_taken_as_the_command_starts() {
    let scratch = Root::new("context-starts");
    let tools = tools(&scratch, TraceContextPropagator::new());

    let calling = {
        let _made = under("1111111111111111").attach();
        tools.execute(call("bash", json!({ "command": PRINTS })))
    };
    let output = calling
        .with_context(under("b7ad6b7169203331"))
        .await
        .unwrap();

    assert_eq!(
        text(output.output),
        format!("00-{TRACE_ID}-b7ad6b7169203331-01|unset|unset|unset\nexit code: 0")
    );
}

/// A config that substitutes a context variable into `tools.builtin.env`
/// withholds it, and the context would put its value back: the parent's
/// trace state and the baggage pass on verbatim.
#[tokio::test]
async fn a_withheld_context_variable_is_not_injected() {
    let scratch = Root::new("context-withheld");
    let mut settings = scratch.settings();
    settings.withheld = ["TRACESTATE".to_owned(), "BAGGAGE".to_owned()].into();
    let tools = BuiltinTools::new(
        settings,
        Arc::new(TextMapCompositePropagator::new(vec![
            Box::new(TraceContextPropagator::new()),
            Box::new(BaggagePropagator::new()),
        ])),
    )
    .unwrap();

    let context = Context::new()
        .with_remote_span_context(SpanContext::new(
            TraceId::from_hex(TRACE_ID).unwrap(),
            SpanId::from_hex("b7ad6b7169203331").unwrap(),
            TraceFlags::SAMPLED,
            true,
            "vendor=secret".parse::<TraceState>().unwrap(),
        ))
        .with_baggage([KeyValue::new("token", "secret")]);
    let text = printed(&tools, PRINTS, context).await;

    assert_eq!(
        text,
        format!("00-{TRACE_ID}-b7ad6b7169203331-01|unset|unset|unset\nexit code: 0")
    );
}
