//! The library root: the config, `build`, and the [`Lablet`] that runs the
//! loop. It selects its adapters through the root kernels it shares with the
//! command line's root, and depends on no other root.
//!
//! A [`Lablet`] is built from a [`Config`] and runs many times. Each run
//! takes a [`RunRequest`] and returns the [`FinishedRun`] the loop made of
//! it, whose summary holds the outcome. Once a run has returned, its
//! telemetry has been flushed to where the config and the environment send
//! it, OTLP to `http://localhost:4318` when neither names another place,
//! and its transcript is written when the config names a place for one.
//!
//! A config's text is read into a [`RawConfig`] first, where an override
//! can state a setting over the text, and then into a [`Config`], which
//! holds `${VAR}` as it's written: the variables are read when the config is
//! checked or a `Lablet` is built, so the digest and every message see the
//! config as it's written. [`check`] checks a config as [`build`] does and
//! stops before the provider is selected. A run stops early when the
//! [`CancelHandle`] its request was given is fired.
//!
//! A `Lablet`'s telemetry is its own: nothing plugs into it, and a host owes
//! it nothing. A host's `tracing` subscriber sees lablet's diagnostics and
//! none of its telemetry, and a tool executor finds its call's span in the
//! current OpenTelemetry context.
//!
//! The process's environment configures it, as it configures any
//! application instrumented with OpenTelemetry. When a `Lablet` is built,
//! or a config checked, the `OTEL_*` variables are read once, and decide
//! where the telemetry goes, and how, wherever the config's `telemetry`
//! section states nothing, while `OTEL_SDK_DISABLED=true` turns it all off
//! whatever the config states. `TRACEPARENT` and `TRACESTATE` are read then
//! too, and name the parent of every run the `Lablet` makes, the same for
//! each, and `BAGGAGE` is in each run's context: a host that runs under a
//! trace of its own has every run beneath that trace's span unless it
//! clears them.
//!
//! A `Lablet` is built and run on a tokio runtime, which its adapters keep
//! their deadlines on.
//!
//! The provider `fake` plays a script in place of a model, so a run needs
//! no key:
//!
//! ```
//! use lablet::{Config, Format, OutcomeDocument, RunId, RunRequest, StopReason};
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
//! telemetry:
//!   file:
//!     path: {directory}/telemetry.otlp.jsonl
//!   otlp:
//!     enabled: false
//! "
//!     ),
//!     Format::Yaml,
//! )?;
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! runtime.block_on(async {
//!     let mut lablet = lablet::build(config).await?;
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
//!     // The file holds both runs, and each run's one wide event.
//!     let telemetry = std::fs::read_to_string(format!("{directory}/telemetry.otlp.jsonl"))?;
//!     let wide_events: Vec<_> = telemetry
//!         .lines()
//!         .filter(|line| line.contains(r#""eventName":"lablet.run""#))
//!         .collect();
//!     assert_eq!(wide_events.len(), 2);
//!     assert!(wide_events[1].contains("the-second-run"));
//!     Ok::<_, Box<dyn std::error::Error>>(())
//! })?;
//! # std::fs::remove_dir_all(directory.to_string())?;
//! # Ok(())
//! # }
//! ```

mod build;
mod clock;
mod lablet;
pub mod telemetry;

pub mod config {
    //! The config: what a `Lablet` is built from, read from YAML or JSON.
    //!
    //! A key the config doesn't know is an error, and that's the whole
    //! policy: nothing is ignored, so nothing a config states is without
    //! effect.
    //!
    //! A path is taken as the config writes it. One that isn't absolute
    //! starts at the working directory, wherever the config's own file is,
    //! so a config read from text and the same config read from a file name
    //! the same files.

    pub use lablet_config::{
        Api, Applied, Builtin, BuiltinTool, CacheScope, Completion, Config, ConfigError, Context,
        Effort, Format, McpLifetime, McpNames, McpResult, McpServer, Model, Otlp, OtlpProtocol,
        OutputCut, Place, Pricing, Prompt, Provider, RawConfig, ResolvedBuiltin, ResolvedConfig,
        ResolvedModel, ResolvedTools, Run, SkillsMode, Telemetry, TelemetryFile, Thinking, Tools,
        TranscriptFormat, schema,
    };
}

pub use build::{build, check};
pub use config::{Config, ConfigError, Format, Place, RawConfig, ResolvedConfig, schema};
pub use lablet::Lablet;
pub use lablet_documents::OutcomeDocument;
pub use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, IdError, RunId, RunLabels, RunOutcome, StopReason,
    ToolSpec,
};
pub use lablet_prepare::{
    BuildError, Checked, ErrorClass, OwnFile, Unsupported, telemetry_on_stderr,
};
pub use lablet_run::FilterList;
pub use lablet_run_request::{CancelHandle, RunIdRefused, RunRequest};

/// The version of lablet, which a run's record names as
/// `gen_ai.agent.version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
