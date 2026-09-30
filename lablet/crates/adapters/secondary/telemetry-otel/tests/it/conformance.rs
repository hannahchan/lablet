//! The cases every observer is held to, of the observer that exports to a
//! file.

use std::path::PathBuf;
use std::sync::Arc;

use lablet_conformance::observer::{
    Subject, a_destination_that_cannot_be_written_changes_nothing_about_the_run,
    a_run_has_exactly_one_wide_event, the_numbers_of_the_wide_event_are_the_sums_of_the_steps,
};
use lablet_conformance::otlp::{Exported, ReadError};
use lablet_run::RunObserver;
use lablet_telemetry_otel::{FileTarget, OtelObserver};
use lablet_test_support::Scratch;

use crate::harness::VERSION;

/// An observer that appends every run to the one file at `path`.
struct Appending {
    observer: OtelObserver,
    path: PathBuf,
}

impl Appending {
    fn to(path: PathBuf) -> Self {
        Self {
            observer: OtelObserver::builder(VERSION)
                .file(FileTarget::Path(path.clone()))
                .build(),
            path,
        }
    }
}

#[async_trait::async_trait]
impl Subject for Appending {
    fn observer(&self) -> Arc<dyn RunObserver> {
        Arc::new(self.observer.clone())
    }

    async fn flush(&self) -> Result<(), String> {
        self.observer
            .flush()
            .await
            .map_err(|error| error.to_string())
    }

    fn exported(&self) -> Result<Exported, ReadError> {
        Exported::read(&self.path)
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_has_exactly_one_wide_event_whatever_its_stop_reason() {
    let scratch = Scratch::new("conformance-one-wide-event");
    let subject = Appending::to(scratch.at("runs.otlp.jsonl"));

    a_run_has_exactly_one_wide_event(&subject).await;

    subject.observer.shutdown().await.unwrap();
    let wide = subject.exported().unwrap();
    assert_eq!(
        wide.records_of("lablet.run").len(),
        2,
        "a shutdown after the runs were flushed adds no wide event"
    );
}

#[tokio::test(start_paused = true)]
async fn the_numbers_of_the_wide_event_are_the_sums_of_the_spans() {
    let scratch = Scratch::new("conformance-sums");
    let subject = Appending::to(scratch.at("runs.otlp.jsonl"));

    the_numbers_of_the_wide_event_are_the_sums_of_the_steps(&subject).await;
}

#[tokio::test(start_paused = true)]
async fn a_file_that_cannot_be_written_changes_nothing_about_the_run() {
    let scratch = Scratch::new("conformance-unwritable");
    let missing = scratch.at("never-made");
    let subject = Appending::to(missing.join("runs.otlp.jsonl"));

    a_destination_that_cannot_be_written_changes_nothing_about_the_run(&subject).await;

    assert!(!missing.exists());
}
