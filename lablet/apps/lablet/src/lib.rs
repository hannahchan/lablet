//! The library root: the config, `build`, and the [`Lablet`] that runs the
//! loop. It selects its adapters through the root kernels it shares with the
//! command line's root, and depends on no other root.
//!
//! A [`Lablet`] is built from a [`Config`] and runs many times. Each run
//! takes a [`RunRequest`] and returns the [`FinishedRun`] the loop made of
//! it, whose summary holds the outcome, and its transcript is written when
//! the config names a place for one.
//!
//! A host hands a `Lablet` a [`Config`] it owns, made in one of two ways.
//! It builds one in code, with the key and each credential `bash` is given
//! set as a [`Secret`]:
//!
//! ```no_run
//! use lablet::config::{BuiltinTool, Provider};
//! use lablet::{Config, Secret};
//! # fn vault(_: &str) -> String { String::new() }
//!
//! let mut config = Config::default();
//! config.model.provider = Provider::Anthropic;
//! config.model.api_key = Some(Secret::new(vault("anthropic")));
//! config.prompt.system = Some("You fix the failing test and stop.".to_owned());
//! config.tools.builtin.root = Some("/srv/task/repo".into());
//! config.tools.builtin.enabled = [BuiltinTool::Bash, BuiltinTool::ReadFile].into();
//! config
//!     .tools
//!     .builtin
//!     .credentials
//!     .insert("GITHUB_TOKEN".to_owned(), Secret::new(vault("github")));
//! ```
//!
//! Or it keeps the command line's file: it reads the file into a
//! [`RawConfig`], where an override can state a setting over the text,
//! fills each `${VAR}` the file writes from wherever it says, through
//! [`RawConfig::substituted`], and places the file's relative paths at the
//! file's directory with [`Config::relative_to`]:
//!
//! ```no_run
//! # fn main() -> Result<(), lablet::ConfigError> {
//! let directory = std::path::Path::new("/srv/task");
//! let config = lablet::RawConfig::from_path(directory.join("lablet.yaml"))?
//!     .substituted(|name| std::env::var_os(name))?
//!     .config()?
//!     .relative_to(directory);
//! # Ok(())
//! # }
//! ```
//!
//! The [`Config`] keeps the record of what the fill changed, so its
//! messages, its resolved form and its digest show each filled setting as
//! the file wrote it. A config that still holds a `${VAR}` is refused when
//! it's read through a [`RawConfig`], and one with a relative path is
//! refused when it's checked unless the host called `relative_to`, since a
//! config made in code or from text has no directory; `Config`'s own
//! `Deserialize` is the in-code path, which fills nothing and counts a
//! credential as set in code. Lablet itself reads no environment variable,
//! though the SDK's builders do: a provider left to its default resource,
//! sampler and limits reads `OTEL_SERVICE_NAME`, `OTEL_TRACES_SAMPLER` and
//! their kin itself, and a batch processor left to its builder's defaults
//! reads `OTEL_BSP_*` or `OTEL_BLRP_*`, so a host that wants none read
//! states them on its builders. The processes a run starts begin from the environment the host hands
//! [`Builder::with_process_environment`], which a config that enables
//! `bash` needs, less every variable that holds one of lablet's
//! credentials. [`Builder::check`] checks a config as [`Builder::build`]
//! does, stops before the provider is selected, and reports the credentials
//! and what `bash` inherits. A run stops early when the [`CancelHandle`]
//! its request was given is fired.
//!
//! A `Lablet` runs on the OpenTelemetry its host provides, and configures
//! no SDK of its own. The host hands in one [`Otel`] of three pieces: the
//! tracer provider its spans go to, the logger provider its records go to,
//! and the propagator a `bash` command's context is injected through. Each
//! is required, and lablet reads none of OpenTelemetry's globals: a host
//! that wants none of a piece hands in the API's no-op for it, and
//! [`Otel::noop`] is the no-op for all three. A host on the global tracer
//! provider hands in `opentelemetry::global::tracer_provider()`, called
//! once it has set its global tracer provider, since the `Otel` takes its
//! tracer when it's made. Each run's root span is the
//! child of the context that's current where the run is awaited, so a run
//! under a span the host has open is in the host's trace, and a command's
//! environment holds its tool span's context and none it would inherit.
//! That context is read when the run's future is first polled, so a host
//! makes its span current on the future itself, with
//! [`FutureExt::with_context`] as the `traced_run` example does, rather
//! than through a guard held on another task or thread.
//! The pieces a host hands in are those of `opentelemetry` 0.33, the
//! version lablet depends on, so a host's OpenTelemetry crates are that
//! version.
//! The host's SDK samples, exports and flushes: a run returns without
//! flushing anything, and [`Lablet::shutdown`] does no telemetry work of
//! its own. So [`Otel::new`] borrows the SDK's providers, and the host
//! keeps them, to flush them and shut them down itself after the last run.
//! A
//! host's `tracing` subscriber sees lablet's diagnostics and none of its
//! telemetry, and a tool executor finds its call's span in the current
//! OpenTelemetry context.
//!
//! [`FutureExt::with_context`]: opentelemetry::trace::FutureExt::with_context
//!
//! What configured an SDK does nothing in library mode, and nothing is said
//! of it: the config's `telemetry.file`, `telemetry.otlp`,
//! `telemetry.propagators` and `telemetry.resource`. No environment
//! variable is read at all: not the `OTEL_*` variables, nor `TRACEPARENT`,
//! `TRACESTATE` and `BAGGAGE` as the run's parent, nor the capture
//! variable, since `telemetry.capture_content` alone decides whether
//! content is captured. Lablet's credentials are dropped from what a
//! command inherits and cut from what it prints as in the command line:
//! `model.api_key`, the values of `telemetry.otlp.headers` and the user
//! information of `telemetry.otlp.endpoint`, though no SDK uses them, and
//! each entry of `tools.builtin.credentials`, which `bash` is given under
//! its name.
//!
//! A `Lablet` is built and run on a tokio runtime, which its adapters keep
//! their deadlines on.
//!
//! The provider `fake` plays a script in place of a model, so a run needs
//! no key. Here a host lends `Otel::new` providers of the SDK's own, over
//! its in-memory exporters, and its own environment for `bash` to start
//! from, checks the config, and shuts its providers down once it has read
//! them:
//!
//! ```
//! use lablet::{Config, Format, Lablet, Otel, OutcomeDocument, RunId, RunRequest, StopReason};
//! use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
//! use opentelemetry_sdk::propagation::TraceContextPropagator;
//! use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let directory = std::env::temp_dir().join(format!("lablet-doctest-{}", std::process::id()));
//! # std::fs::create_dir_all(directory.join("work"))?;
//! # let directory = directory.display();
//! std::fs::write(
//!     format!("{directory}/script.yaml"),
//!     "
//! - response:
//!     content:
//!       - text: I'll look at what's there.
//!       - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
//!     finish: tool_use
//!     usage: { input_tokens: 120, output_tokens: 30 }
//! - response:
//!     content:
//!       - text: There's one file, parser.rs.
//!     finish: end_turn
//!     usage: { input_tokens: 180, output_tokens: 12 }
//! ",
//! )?;
//! let config = Config::from_str(
//!     &format!(
//!         "
//! model:
//!   provider: fake
//!   script: {directory}/script.yaml
//!   name: scripted-1
//! prompt:
//!   system: You answer tersely.
//! tools:
//!   builtin:
//!     root: {directory}/work
//!     enabled: [bash]
//! "
//!     ),
//!     Format::Yaml,
//! )?;
//! let spans = InMemorySpanExporter::default();
//! let records = InMemoryLogExporter::default();
//! let tracer_provider = SdkTracerProvider::builder()
//!     .with_simple_exporter(spans.clone())
//!     .build();
//! let logger_provider = SdkLoggerProvider::builder()
//!     .with_simple_exporter(records.clone())
//!     .build();
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! runtime.block_on(async {
//!     let otel = Otel::new(
//!         &tracer_provider,
//!         &logger_provider,
//!         TraceContextPropagator::new(),
//!     );
//!     let builder = Lablet::builder(config, otel).with_process_environment(std::env::vars_os());
//!     let checked = builder.check().await?;
//!     let mut lablet = builder.build().await?;
//!     assert_eq!(checked.tools(), lablet.tools());
//!
//!     let first = lablet.run(RunRequest::new("What's in the directory?")?).await;
//!     let named = RunId::new("the-second-run")?;
//!     let second = lablet
//!         .run(RunRequest::new("What's in the directory?")?.run_id(named.clone())?)
//!         .await;
//!     lablet.shutdown().await;
//!
//!     // Every run of one `Lablet` hears the script from its first entry.
//!     let (first, second) = (first.summary.outcome, second.summary.outcome);
//!     for outcome in [&first, &second] {
//!         assert_eq!(outcome.stop_reason(), StopReason::Completed);
//!         assert_eq!(outcome.result().text, "There's one file, parser.rs.");
//!         assert_eq!((outcome.turns, outcome.tool_calls), (2, 1));
//!         assert_eq!(outcome.usage.total(), 342);
//!     }
//!     assert_eq!(second.run_id, named);
//!     assert_ne!(first.run_id, second.run_id);
//!
//!     // The outcome as a document, which is what the two runs share once
//!     // what names a run and what times it are taken out.
//!     let shared = |outcome| {
//!         let mut document = serde_json::to_value(OutcomeDocument::from(outcome))?;
//!         for key in ["run_id", "duration_ms"] {
//!             document.as_object_mut().and_then(|keys| keys.remove(key));
//!         }
//!         Ok::<_, serde_json::Error>(document)
//!     };
//!     assert_eq!(shared(first)?, shared(second)?);
//!
//!     // The host's providers hold both runs, and each run's one wide event.
//!     let wide_events: Vec<_> = records
//!         .get_emitted_logs()?
//!         .into_iter()
//!         .filter(|log| log.record.event_name() == Some("lablet.run"))
//!         .collect();
//!     assert_eq!(wide_events.len(), 2);
//!     let roots = spans
//!         .get_finished_spans()?
//!         .into_iter()
//!         .filter(|span| span.name.starts_with("invoke_agent"))
//!         .count();
//!     assert_eq!(roots, 2);
//!     Ok::<_, Box<dyn std::error::Error>>(())
//! })?;
//! // Read, so they can go: the in-memory exporters forget what they hold
//! // when their providers shut down.
//! tracer_provider.shutdown()?;
//! logger_provider.shutdown()?;
//! # std::fs::remove_dir_all(directory.to_string())?;
//! # Ok(())
//! # }
//! ```

mod build;
mod lablet;
mod otel;
pub mod telemetry;

pub mod config {
    //! The config: what a `Lablet` is built from, read from YAML or JSON.
    //!
    //! A key the config doesn't know is an error, and that's the whole
    //! policy: nothing is ignored, so nothing a config states is without
    //! effect.
    //!
    //! A path is taken as the config writes it. One that isn't absolute is
    //! refused when the config is checked unless the host called
    //! [`Config::relative_to`], since a config made in code or from text
    //! has no directory. That places each relative path the file wrote at
    //! a directory, the file's own for a config read from a file; a path an
    //! override gave stays as written and starts where the process runs.
    //!
    //! A `${VAR}` is filled by the host, through [`RawConfig::substituted`],
    //! before the config is typed; the record of what the fill changed
    //! travels on the config as [`Filled`].

    pub use lablet_config::{
        Api, Applied, Builtin, BuiltinTool, CacheScope, Completion, Config, ConfigError, Context,
        Effort, Fill, Filled, Format, Headers, KeyPath, Masked, McpLifetime, McpNames, McpResult,
        McpServer, Model, Otlp, OtlpProtocol, OutputCut, Place, Places, Pricing, Prompt,
        Propagator, Propagators, Provider, RawConfig, Refusal, ResolvedBuiltin, ResolvedConfig,
        ResolvedModel, ResolvedTools, Resource, Run, Secret, Setting, SkillsMode, Telemetry,
        TelemetryFile, Thinking, Tls, Tools, TranscriptFormat, Variable, WrittenBack,
        is_credential, schema, shown,
    };
}

pub use build::{Builder, build};
pub use config::{
    Config, ConfigError, Format, Place, RawConfig, Refusal, ResolvedConfig, Secret, schema,
};
pub use lablet::Lablet;
pub use lablet_documents::OutcomeDocument;
pub use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, IdError, RunId, RunLabels, RunOutcome, StopReason,
    ToolSpec,
};
pub use lablet_prepare::{
    BuildError, Checked, Environment, ErrorClass, OwnFile, Passed, Unsupported,
};
pub use lablet_run::FilterList;
pub use lablet_run_request::{CancelHandle, RunIdRefused, RunRequest};
pub use lablet_secrets::{Credential, Held, Source};
pub use otel::Otel;

/// The version of lablet, which a run's record names as
/// `gen_ai.agent.version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
