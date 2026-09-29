//! Where a run's telemetry goes, and what a destination that can't be
//! written changes: nothing about the run.

use lablet_conformance::otlp::Exported;
use lablet_telemetry_otel::FileTarget;
use lablet_telemetry_registry::attribute as key;

use crate::harness::{FAILS_CALLS_ENDS, Harness, OTHER_RUN, RUN, Scratch, Settings};

fn runs_of(exported: &Exported) -> Vec<&str> {
    exported
        .records_of("lablet.run")
        .into_iter()
        .map(|wide| {
            wide.attributes[key::GEN_AI_CONVERSATION_ID]
                .as_str()
                .unwrap()
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_file_of_a_run_is_whole_when_the_flush_after_the_run_returns() {
    let scratch = Scratch::new("whole");
    let mut harness = Harness::playing(FAILS_CALLS_ENDS, Settings::in_scratch(&scratch)).await;

    harness.run(RUN).await;
    harness.observer.flush().await.unwrap();

    let exported = Exported::read(&scratch.file_of(RUN)).unwrap();
    assert_eq!(exported.spans.len(), 8);
    assert_eq!(runs_of(&exported), [RUN]);
    assert_eq!(exported.records.last().unwrap().event_name, "lablet.run");
    assert_eq!(exported.records.last().unwrap().line, exported.lines);
}

#[tokio::test(start_paused = true)]
async fn two_runs_of_one_observer_have_a_file_each() {
    let scratch = Scratch::new("two-files");
    let mut harness = Harness::playing(FAILS_CALLS_ENDS, Settings::in_scratch(&scratch)).await;

    let first = harness.run(RUN).await;
    harness.observer.flush().await.unwrap();
    harness.provider.rewind();
    let second = harness.run(OTHER_RUN).await;
    harness.observer.flush().await.unwrap();

    assert_eq!(first.summary.outcome.usage, second.summary.outcome.usage);
    let files = [RUN, OTHER_RUN].map(|run| Exported::read(&scratch.file_of(run)).unwrap());
    for (file, run) in files.iter().zip([RUN, OTHER_RUN]) {
        assert_eq!(file.spans.len(), 8);
        assert_eq!(runs_of(file), [run]);
        for span in &file.spans {
            assert_eq!(span.attributes[key::GEN_AI_CONVERSATION_ID], run);
        }
    }
    assert_ne!(files[0].spans[0].trace_id, files[1].spans[0].trace_id);
    assert_eq!(std::fs::read_dir(scratch.directory()).unwrap().count(), 2);
}

#[tokio::test(start_paused = true)]
async fn two_runs_of_one_observer_are_appended_to_the_one_file_it_was_given() {
    let scratch = Scratch::new("one-file");
    let path = scratch.directory().join("runs.otlp.jsonl");
    let mut harness = Harness::playing(
        FAILS_CALLS_ENDS,
        Settings {
            target: FileTarget::Path(path.clone()),
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    harness.run(RUN).await;
    harness.observer.flush().await.unwrap();
    let after_the_first = Exported::read(&path).unwrap();
    harness.provider.rewind();
    harness.run(OTHER_RUN).await;
    harness.observer.shutdown().await.unwrap();

    let after_both = Exported::read(&path).unwrap();
    assert_eq!(after_the_first.spans.len(), 8);
    assert_eq!(after_both.spans[..8], after_the_first.spans[..]);
    assert_eq!(after_both.spans.len(), 16);
    assert_eq!(runs_of(&after_both), [RUN, OTHER_RUN]);
    assert_eq!(after_both.records.last().unwrap().line, after_both.lines);
}

#[tokio::test(start_paused = true)]
async fn a_file_that_was_moved_after_a_run_holds_that_run_and_its_path_the_run_after() {
    let scratch = Scratch::new("moved-file");
    let path = scratch.directory().join("runs.otlp.jsonl");
    let moved = scratch.directory().join("first.otlp.jsonl");
    let mut harness = Harness::playing(
        FAILS_CALLS_ENDS,
        Settings {
            target: FileTarget::Path(path.clone()),
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    harness.run(RUN).await;
    harness.observer.flush().await.unwrap();
    std::fs::rename(&path, &moved).unwrap();
    harness.provider.rewind();
    harness.run(OTHER_RUN).await;
    harness.observer.shutdown().await.unwrap();

    let (moved, at_the_path) = (
        Exported::read(&moved).unwrap(),
        Exported::read(&path).unwrap(),
    );
    for (file, run) in [(&moved, RUN), (&at_the_path, OTHER_RUN)] {
        assert_eq!(runs_of(file), [run]);
        assert_eq!(file.spans.len(), 8, "{run}");
        for span in &file.spans {
            assert_eq!(span.attributes[key::GEN_AI_CONVERSATION_ID], run);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_destination_that_cannot_be_written_changes_nothing_about_the_run() {
    let scratch = Scratch::new("unwritable");
    let mut written = Harness::playing(FAILS_CALLS_ENDS, Settings::in_scratch(&scratch)).await;
    let missing = scratch.directory().join("never-made");
    let mut unwritable = Harness::playing(
        FAILS_CALLS_ENDS,
        Settings {
            target: FileTarget::EachRun {
                directory: missing.clone(),
            },
            capture_content: true,
            ..Settings::in_scratch(&scratch)
        },
    )
    .await;

    let expected = written.run(RUN).await;
    written.observer.flush().await.unwrap();
    let finished = unwritable.run(RUN).await;
    let flushed = unwritable.observer.flush().await;

    assert_eq!(finished.summary.outcome, expected.summary.outcome);
    assert_eq!(finished.transcript, expected.transcript);
    assert_eq!(
        finished.summary.provider.latency.total_ms(),
        expected.summary.provider.latency.total_ms()
    );
    let failures = flushed.unwrap_err();
    let queues: Vec<_> = failures
        .failures()
        .iter()
        .map(|failure| failure.split(": ").next().unwrap())
        .collect();
    assert_eq!(queues, ["spans", "log records", "the wide event"]);
    assert!(
        failures.to_string().contains(&format!(
            "{} couldn't be written: ",
            missing.join(format!("lablet-{RUN}.otlp.jsonl")).display()
        )),
        "{failures}"
    );
    assert!(!missing.exists());
    assert_eq!(unwritable.observer.shutdown().await, Ok(()));
}
