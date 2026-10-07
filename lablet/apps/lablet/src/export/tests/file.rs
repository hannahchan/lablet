//! Where a run's telemetry goes in a file, and what a destination that
//! can't be written reports.

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;

use super::harness::{
    CONTENT, CONTENT_PER_RUN, OTHER_RUN, RUN, RUN_KEY, Records, SPANS_PER_RUN, Settings, WIDE,
    built, emit_run, file_of, providers, run, runs_of,
};
use crate::export::FileTarget;

#[tokio::test]
async fn the_file_of_a_run_is_whole_when_the_flush_after_the_run_returns() {
    let scratch = Scratch::new("whole");
    let telemetry = built(Settings::in_scratch(&scratch));

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let exported = Exported::read(&file_of(&scratch, RUN)).unwrap();
    assert_eq!(exported.spans.len(), SPANS_PER_RUN);
    assert_eq!(exported.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(runs_of(&exported), [RUN]);
    assert_eq!(exported.records_of(WIDE).len(), 1);
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn two_runs_of_one_telemetry_have_a_file_each() {
    let scratch = Scratch::new("two-files");
    let telemetry = built(Settings::in_scratch(&scratch));

    run(&telemetry, RUN, Records::Exception).await.unwrap();
    run(&telemetry, OTHER_RUN, Records::Exception)
        .await
        .unwrap();

    let files = [RUN, OTHER_RUN].map(|run| Exported::read(&file_of(&scratch, run)).unwrap());
    for (file, run) in files.iter().zip([RUN, OTHER_RUN]) {
        assert_eq!(file.spans.len(), SPANS_PER_RUN);
        assert_eq!(runs_of(file), [run]);
        for span in &file.spans {
            assert_eq!(span.attributes[RUN_KEY], run);
        }
        for record in &file.records {
            assert_eq!(record.attributes[RUN_KEY], run);
        }
    }
    assert_ne!(files[0].spans[0].trace_id, files[1].spans[0].trace_id);
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 2);
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn two_runs_of_one_telemetry_are_appended_to_the_one_file_it_was_given() {
    let scratch = Scratch::new("one-file");
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        ..Settings::in_scratch(&scratch)
    });

    run(&telemetry, RUN, Records::Exception).await.unwrap();
    let after_the_first = Exported::read(&path).unwrap();
    run(&telemetry, OTHER_RUN, Records::Exception)
        .await
        .unwrap();
    telemetry.shutdown().await.unwrap();

    let after_both = Exported::read(&path).unwrap();
    assert_eq!(after_the_first.spans.len(), SPANS_PER_RUN);
    assert_eq!(after_both.spans[..SPANS_PER_RUN], after_the_first.spans[..]);
    assert_eq!(after_both.spans.len(), 2 * SPANS_PER_RUN);
    assert_eq!(runs_of(&after_both), [RUN, OTHER_RUN]);
}

#[tokio::test]
async fn a_file_that_was_moved_after_a_run_holds_that_run_and_its_path_the_run_after() {
    let scratch = Scratch::new("moved-file");
    let path = scratch.at("runs.otlp.jsonl");
    let moved = scratch.at("first.otlp.jsonl");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.clone())),
        ..Settings::in_scratch(&scratch)
    });

    run(&telemetry, RUN, Records::Exception).await.unwrap();
    std::fs::rename(&path, &moved).unwrap();
    run(&telemetry, OTHER_RUN, Records::Exception)
        .await
        .unwrap();
    telemetry.shutdown().await.unwrap();

    let (moved, at_the_path) = (
        Exported::read(&moved).unwrap(),
        Exported::read(&path).unwrap(),
    );
    for (file, run) in [(&moved, RUN), (&at_the_path, OTHER_RUN)] {
        assert_eq!(runs_of(file), [run]);
        assert_eq!(file.spans.len(), SPANS_PER_RUN, "{run}");
        for span in &file.spans {
            assert_eq!(span.attributes[RUN_KEY], run);
        }
    }
}

#[tokio::test]
async fn a_destination_that_cannot_be_written_fails_both_flushes_and_names_no_path() {
    let scratch = Scratch::new("unwritable");
    let missing = scratch.at("never-made");
    let telemetry = built(Settings {
        target: Some(FileTarget::EachRun {
            directory: missing.clone(),
        }),
        ..Settings::in_scratch(&scratch)
    });

    let flushed = run(&telemetry, RUN, Records::Captured).await;

    let failures = flushed.unwrap_err();
    assert_eq!(providers(&failures), ["spans", "log records"]);
    let failures = failures.to_string();
    assert!(
        failures.contains("the telemetry file couldn't be written: "),
        "{failures}"
    );
    assert!(
        !failures.contains(&missing.display().to_string()),
        "{failures}"
    );
    assert!(!missing.exists());
    assert_eq!(telemetry.shutdown().await, Ok(()));
}

/// Two `Lablet`s in one process each build a telemetry of their own, and
/// nothing is read from the global providers, so each tracer and logger
/// reaches its own destinations and no other's.
#[tokio::test]
async fn two_telemetries_in_one_process_each_export_only_what_their_own_tracer_and_logger_emitted()
{
    let (first, second) = (Scratch::new("two-first"), Scratch::new("two-second"));
    let (one, other) = (
        built(Settings::in_scratch(&first)),
        built(Settings::in_scratch(&second)),
    );

    // Both runs are in flight before either is flushed, so a signal that
    // reached the wrong destination would be in its file.
    let wide_of_one = emit_run(&one, RUN, Records::Captured);
    let wide_of_other = emit_run(&other, OTHER_RUN, Records::Captured);
    one.flush(wide_of_one).await.unwrap();
    other.flush(wide_of_other).await.unwrap();

    let of_one = Exported::read(&file_of(&first, RUN)).unwrap();
    let of_other = Exported::read(&file_of(&second, OTHER_RUN)).unwrap();
    for (exported, run) in [(&of_one, RUN), (&of_other, OTHER_RUN)] {
        assert_eq!(exported.spans.len(), SPANS_PER_RUN, "{run}");
        assert_eq!(exported.records.len(), 1 + CONTENT_PER_RUN + 1, "{run}");
        assert_eq!(runs_of(exported), [run]);
        for span in &exported.spans {
            assert_eq!(span.attributes[RUN_KEY], run);
        }
        for record in &exported.records {
            assert_eq!(record.attributes[RUN_KEY], run);
        }
    }
    assert_ne!(of_one.spans[0].trace_id, of_other.spans[0].trace_id);
    assert_eq!(std::fs::read_dir(first.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(second.path()).unwrap().count(), 1);
    one.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}
