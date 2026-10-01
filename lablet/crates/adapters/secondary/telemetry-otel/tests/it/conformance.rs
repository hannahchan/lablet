//! The cases every observer is held to, of the observer that exports to a
//! file, to a collector over the network by either transport, and to both.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lablet_conformance::observer::{
    Subject, a_destination_that_cannot_be_written_changes_nothing_about_the_run,
    a_destination_that_never_answers_holds_the_flush_only_for_its_bound,
    a_run_has_exactly_one_wide_event, the_destinations_of_one_run_hold_the_same_spans_and_records,
    the_numbers_of_the_wide_event_are_the_sums_of_the_steps,
};
use lablet_conformance::otlp::{Exported, ReadError};
use lablet_conformance::receiver::{Mode, Receiver};
use lablet_run::RunObserver;
use lablet_telemetry_otel::{FileTarget, OtelObserver, OtlpSettings, Transport};
use lablet_test_support::Scratch;

use crate::harness::VERSION;

/// How long the network subjects wait for a flush: enough for a collector
/// that answers, and short enough that a hang is held before the SDK's own
/// five seconds would be.
const FLUSH_BOUND: Duration = Duration::from_millis(500);

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
                .build()
                .unwrap(),
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

/// The endpoint of `receiver` that `transport` speaks to.
fn endpoint_of(receiver: &Receiver, transport: Transport) -> String {
    match transport {
        Transport::Grpc => receiver.grpc_endpoint(),
        Transport::HttpProtobuf => receiver.http_endpoint(),
    }
}

fn settings(transport: Transport, endpoint: String) -> OtlpSettings {
    OtlpSettings {
        transport,
        endpoint: Some(endpoint),
        headers: Vec::new(),
        strip_environment_headers: true,
    }
}

/// An observer that exports to the in-process receiver alone, by one
/// transport.
struct Network {
    observer: OtelObserver,
    receiver: Receiver,
}

impl Network {
    async fn over(transport: Transport, mode: Mode) -> Self {
        let receiver = Receiver::start(mode).await;
        let observer = OtelObserver::builder(VERSION)
            .otlp(settings(transport, endpoint_of(&receiver, transport)))
            .flush_timeout(FLUSH_BOUND)
            .build()
            .unwrap();
        Self { observer, receiver }
    }
}

#[async_trait::async_trait]
impl Subject for Network {
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
        self.receiver.exported()
    }
}

/// An observer that exports to the receiver and to a file at once. The
/// file is what it reads back, so a case holds the file to the run while
/// the network does as it does.
struct Both {
    observer: OtelObserver,
    receiver: Receiver,
    path: PathBuf,
}

impl Both {
    async fn over(transport: Transport, mode: Mode, path: PathBuf) -> Self {
        let receiver = Receiver::start(mode).await;
        let observer = OtelObserver::builder(VERSION)
            .file(FileTarget::Path(path.clone()))
            .otlp(settings(transport, endpoint_of(&receiver, transport)))
            .flush_timeout(FLUSH_BOUND)
            .build()
            .unwrap();
        Self {
            observer,
            receiver,
            path,
        }
    }
}

#[async_trait::async_trait]
impl Subject for Both {
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

// The file

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

// The network, by either transport (O7)

#[tokio::test(start_paused = true)]
async fn a_run_over_grpc_has_exactly_one_wide_event_whatever_its_stop_reason() {
    let subject = Network::over(Transport::Grpc, Mode::Answers).await;

    a_run_has_exactly_one_wide_event(&subject).await;

    subject.observer.shutdown().await.unwrap();
    assert_eq!(
        subject.exported().unwrap().records_of("lablet.run").len(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_over_http_has_exactly_one_wide_event_whatever_its_stop_reason() {
    let subject = Network::over(Transport::HttpProtobuf, Mode::Answers).await;

    a_run_has_exactly_one_wide_event(&subject).await;

    subject.observer.shutdown().await.unwrap();
    assert_eq!(
        subject.exported().unwrap().records_of("lablet.run").len(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn the_numbers_of_the_wide_event_over_grpc_are_the_sums_of_the_spans() {
    let subject = Network::over(Transport::Grpc, Mode::Answers).await;

    the_numbers_of_the_wide_event_are_the_sums_of_the_steps(&subject).await;

    subject.observer.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_numbers_of_the_wide_event_over_http_are_the_sums_of_the_spans() {
    let subject = Network::over(Transport::HttpProtobuf, Mode::Answers).await;

    the_numbers_of_the_wide_event_are_the_sums_of_the_steps(&subject).await;

    subject.observer.shutdown().await.unwrap();
}

// An endpoint nothing listens on (O3)

async fn a_closed_port_changes_nothing_about_the_run(transport: Transport) {
    let closed = Receiver::closed();
    let subject = Network {
        observer: OtelObserver::builder(VERSION)
            .otlp(settings(transport, format!("http://{closed}")))
            .build()
            .unwrap(),
        receiver: Receiver::start(Mode::Answers).await,
    };

    a_destination_that_cannot_be_written_changes_nothing_about_the_run(&subject).await;

    assert!(subject.receiver.requests().is_empty());
    let shut = subject.observer.shutdown().await;
    assert_eq!(
        shut,
        Ok(()),
        "the flush reported every failure, and left nothing for the shutdown to"
    );
}

#[tokio::test(start_paused = true)]
async fn a_grpc_endpoint_nothing_listens_on_changes_nothing_about_the_run() {
    a_closed_port_changes_nothing_about_the_run(Transport::Grpc).await;
}

#[tokio::test(start_paused = true)]
async fn an_http_endpoint_nothing_listens_on_changes_nothing_about_the_run() {
    a_closed_port_changes_nothing_about_the_run(Transport::HttpProtobuf).await;
}

// A collector that accepts and never answers, with the file on (O16)

async fn a_receiver_that_never_answers_leaves_the_file_whole(transport: Transport, test: &str) {
    let scratch = Scratch::new(test);
    let path = scratch.at("runs.otlp.jsonl");
    let subject = Both::over(transport, Mode::NeverAnswers, path).await;

    let said =
        a_destination_that_never_answers_holds_the_flush_only_for_its_bound(&subject, FLUSH_BOUND)
            .await;

    assert_eq!(
        said, "telemetry wasn't exported whole: otlp: the flush didn't end within 500ms",
        "the network destination alone is named, by its bound, and the file isn't"
    );
    let file = subject.exported().unwrap();
    assert_eq!(file.records.last().unwrap().event_name, "lablet.run");
    assert_eq!(file.records.last().unwrap().line, file.lines);
    assert_eq!(file.records_of("lablet.run").len(), 1);
    assert!(
        !file.spans.is_empty(),
        "the file holds the run's spans beside its wide event"
    );
    assert!(
        subject
            .receiver
            .exported()
            .unwrap()
            .records_of("lablet.run")
            .is_empty(),
        "the receiver that never answered holds no wide event"
    );
}

#[tokio::test(start_paused = true)]
async fn a_grpc_receiver_that_never_answers_holds_the_flush_for_its_bound_and_the_file_is_whole() {
    a_receiver_that_never_answers_leaves_the_file_whole(Transport::Grpc, "conformance-hang-grpc")
        .await;
}

#[tokio::test(start_paused = true)]
async fn an_http_receiver_that_never_answers_holds_the_flush_for_its_bound_and_the_file_is_whole() {
    a_receiver_that_never_answers_leaves_the_file_whole(
        Transport::HttpProtobuf,
        "conformance-hang-http",
    )
    .await;
}

// The file and the network hold the same run (O9)

async fn the_file_and_the_receiver_hold_the_same_run(transport: Transport, test: &str) {
    let scratch = Scratch::new(test);
    let subject = Both::over(transport, Mode::Answers, scratch.at("runs.otlp.jsonl")).await;

    the_destinations_of_one_run_hold_the_same_spans_and_records(&subject, &|| {
        subject.receiver.exported()
    })
    .await;

    subject.observer.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_file_and_the_grpc_receiver_hold_the_same_spans_and_records() {
    the_file_and_the_receiver_hold_the_same_run(Transport::Grpc, "conformance-same-grpc").await;
}

#[tokio::test(start_paused = true)]
async fn the_file_and_the_http_receiver_hold_the_same_spans_and_records() {
    the_file_and_the_receiver_hold_the_same_run(Transport::HttpProtobuf, "conformance-same-http")
        .await;
}
