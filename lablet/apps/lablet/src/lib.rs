//! The composition root as a library: the config, `build`, and the
//! [`Lablet`] that runs the loop. It's the one place that selects adapters.
//!
//! A [`Lablet`] is built from a [`Config`] and runs many times. Each run
//! takes a [`RunRequest`] and returns the [`FinishedRun`] the loop made of
//! it, whose summary holds the outcome. Once a run has returned, its
//! telemetry is in the file the config names, as OTLP/JSON lines, and its
//! transcript is written when the config names a place for one.
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
//!     // The file holds both runs, and the last line of each is its wide
//!     // event.
//!     let telemetry = std::fs::read_to_string(format!("{directory}/telemetry.otlp.jsonl"))?;
//!     let wide_events = telemetry
//!         .lines()
//!         .filter(|line| line.contains(r#""eventName":"lablet.run""#));
//!     assert_eq!(wide_events.count(), 2);
//!     assert!(telemetry.lines().last().is_some_and(|line| line.contains("the-second-run")));
//!     Ok::<_, Box<dyn std::error::Error>>(())
//! })?;
//! # std::fs::remove_dir_all(directory.to_string())?;
//! # Ok(())
//! # }
//! ```

mod build;
mod cancel;
mod clock;
pub mod config;
mod lablet;
mod otlp;
mod root;
mod root_span;
mod secrets;
mod settings;
pub mod telemetry;
mod wide;

pub use build::{BuildError, Checked, ErrorClass, Unsupported, build, check, telemetry_on_stderr};
pub use cancel::CancelHandle;
pub use config::{Config, ConfigError, Format, Place, RawConfig, ResolvedConfig, schema};
pub use lablet::{Lablet, RunIdRefused, RunRequest};
pub use lablet_documents::OutcomeDocument;
pub use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, IdError, RunId, RunLabels, RunOutcome, StopReason,
    ToolSpec,
};
pub use lablet_run::FilterList;
pub use root::OwnFile;

/// The version of lablet, which a run's record names as
/// `gen_ai.agent.version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
