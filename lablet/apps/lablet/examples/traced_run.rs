//! A host with an OpenTelemetry SDK of its own runs lablet as a library:
//! it hands in its tracer and logger providers, opens a span of its own,
//! and makes a traced run beneath it that needs no model and no key. A
//! script is played in place of the model, `bash` works under a root of its
//! own, and the run leaves its transcript in a file.
//!
//! The host's SDK sends over OTLP/HTTP to `http://localhost:4318`, or to
//! where `OTEL_EXPORTER_OTLP_ENDPOINT` says, as the SDK reads it, so the
//! run is found beneath the host's span in a collector such as the
//! examples' Jaeger. Lablet configures nothing of it.
//!
//! ```bash
//! cargo run -p lablet --example traced_run
//! ```

use std::error::Error;

use lablet::{Config, Format, Lablet, OutcomeDocument, RunLabels, RunRequest};
use opentelemetry::Context;
use opentelemetry::trace::{
    FutureExt as _, Span as _, TraceContextExt as _, Tracer as _, TracerProvider as _,
};
use opentelemetry_otlp::{LogExporter, Protocol, SpanExporter, WithExportConfig as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

const SCRIPT: &str = "
- response:
    content:
      - text: I'll look at what's there.
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls -a } } }
    finish: tool_use
    usage: { input_tokens: 120, output_tokens: 30 }
    latency: 40ms
- response:
    content:
      - text: The directory is empty.
    finish: end_turn
    usage: { input_tokens: 180, output_tokens: 12 }
    latency: 25ms
";

/// The host's own name, on its resource and its tracer.
const HOST: &str = "traced-run-host";

#[tokio::main]
#[expect(
    clippy::print_stdout,
    reason = "an example says what the run came to, where its trace is, and where it left its transcript"
)]
async fn main() -> Result<(), Box<dyn Error>> {
    let directory = std::env::temp_dir().join("lablet-traced-run");
    std::fs::create_dir_all(directory.join("work"))?;
    let script = directory.join("script.yaml");
    std::fs::write(&script, SCRIPT)?;

    let config = Config::from_str(
        &format!(
            "
model:
  provider: fake
  script: {script}
  name: scripted-1
prompt:
  system: You answer tersely.
run:
  transcript_path: {directory}/transcript-{{run_id}}.json
tools:
  builtin:
    root: {directory}/work
    enabled: [bash]
telemetry:
  capture_content: true
",
            script = script.display(),
            directory = directory.display(),
        ),
        Format::Yaml,
    )?;

    // The host's SDK, which samples, exports and flushes what lablet emits.
    let resource = Resource::builder().with_service_name(HOST).build();
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(
            SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .build()?,
        )
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(
            LogExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .build()?,
        )
        .build();

    let mut lablet = Lablet::builder(config, logger_provider.clone())
        .with_tracer_provider(tracer_provider.clone())
        .build()
        .await?;
    let request = RunRequest::new("What's in the directory?")?.labels(RunLabels {
        task: Some("list-the-directory".to_owned()),
        ..RunLabels::default()
    });
    let operation = tracer_provider.tracer(HOST).start("host operation");
    let trace_id = operation.span_context().trace_id();
    let within = Context::current_with_span(operation);
    let finished = lablet.run(request).with_context(within.clone()).await;
    within.span().end();
    lablet.shutdown().await;
    // The batch processors wait for the collector on threads of their own.
    tokio::task::spawn_blocking(move || {
        let traces = tracer_provider.shutdown();
        let logs = logger_provider.shutdown();
        traces.and(logs)
    })
    .await??;

    let outcome = finished.summary.outcome;
    let transcript = directory.join(format!("transcript-{}.json", outcome.run_id));
    println!(
        "{}",
        serde_json::to_string_pretty(&OutcomeDocument::from(outcome))?
    );
    println!();
    println!("trace:      {trace_id}");
    println!("transcript: {}", transcript.display());
    Ok(())
}
