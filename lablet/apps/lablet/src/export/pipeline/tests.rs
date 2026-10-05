use std::time::{Instant, UNIX_EPOCH};

use opentelemetry::KeyValue;
use opentelemetry::logs::{LogRecord as _, Logger as _, LoggerProvider as _};
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};

use super::*;
use crate::export::testing::memory::{Export, Memory};

const TRACE: TraceId = TraceId::from_bytes([0xab; 16]);

fn resource() -> Resource {
    Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "lablet"))
        .build()
}

fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet").build()
}

/// A span as the SDK hands one over when it ends, with nothing of lablet's.
fn span(number: u64) -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TRACE,
            SpanId::from(number + 1),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Client,
        name: format!("chat {number}").into(),
        start_time: UNIX_EPOCH,
        end_time: UNIX_EPOCH,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: scope(),
    }
}

/// Emits `records` log records to `queue`, as a logger provider does.
fn emit(queue: &RecordQueue, records: usize) {
    let provider = SdkLoggerProvider::builder().build();
    let logger = provider.logger_with_scope(scope());
    for _ in 0..records {
        let mut record = logger.create_log_record();
        record.set_event_name("lablet.test");
        record.set_timestamp(UNIX_EPOCH);
        queue.emit(&mut record, &scope());
    }
}

fn lost() -> Arc<Lost> {
    Arc::new(Lost::default())
}

// The room in a queue

#[test]
fn a_queue_has_room_for_as_many_as_its_capacity_and_the_next_is_lost() {
    let lost = lost();
    let room = Room::new(2, Arc::clone(&lost));

    let admitted = [room.admit(), room.admit(), room.admit(), room.admit()];

    assert_eq!(admitted, [true, true, false, false]);
    assert_eq!(lost.take(), 2);
}

#[test]
fn what_leaves_a_queue_gives_its_room_back() {
    let lost = lost();
    let room = Room::new(2, Arc::clone(&lost));
    assert!(room.admit() && room.admit());

    room.left(1, true, room.generation());

    assert_eq!([room.admit(), room.admit()], [true, false]);
    room.left(2, true, room.generation());
    assert_eq!(
        [room.admit(), room.admit(), room.admit()],
        [true, true, false]
    );
    assert_eq!(lost.take(), 2);
}

#[test]
fn what_an_export_that_failed_held_is_lost_and_its_room_is_given_back() {
    let lost = lost();
    let room = Room::new(3, Arc::clone(&lost));
    assert!(room.admit() && room.admit() && room.admit());

    room.left(2, false, room.generation());

    assert_eq!(lost.take(), 2);
    assert_eq!(
        [room.admit(), room.admit(), room.admit()],
        [true, true, false]
    );
}

#[test]
fn a_count_that_was_taken_starts_again_from_none() {
    let lost = lost();
    lost.add(lost.generation(), 3);
    lost.add(lost.generation(), 4);

    assert_eq!(lost.take(), 7);
    assert_eq!(lost.take(), 0);
    lost.add(lost.generation(), 1);
    assert_eq!(lost.take(), 1);
}

#[test]
fn a_loss_of_a_generation_that_was_taken_is_counted_against_no_run() {
    let lost = lost();
    let before = lost.generation();
    lost.add(before, 2);

    assert_eq!(lost.take(), 2);
    lost.add(before, 3);
    let now = lost.generation();
    lost.add(now, 1);

    assert_ne!(before, now);
    assert_eq!(
        lost.take(),
        1,
        "the late loss isn't in the next run's count"
    );
    assert_eq!(lost.take(), 0, "and isn't carried anywhere");
}

#[test]
fn an_export_that_fails_after_its_runs_count_was_taken_is_counted_against_no_run() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    memory.hold();
    memory.refuse_spans(true);
    queue.on_end(span(0));
    // The export of the span is in flight, held by the destination, when
    // the run's count is taken.
    memory.wait_until_exporting();
    let of_the_run = lost.take();

    memory.release();
    // The flush is answered once the export in flight has failed: the
    // queue's thread does one thing at a time.
    let flushed = queue.force_flush();

    assert_eq!(of_the_run, 0);
    flushed.unwrap();
    assert!(memory.exported_spans().is_empty(), "the export was refused");
    assert_eq!(
        lost.take(),
        0,
        "the loss landed after the run's wide event was made, so no run counts it"
    );
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
}

// Spans

#[test]
fn a_queue_of_spans_exports_what_it_holds_when_it_is_flushed() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new("memory", memory.spans(), &resource(), &lost);

    queue.on_end(span(0));
    queue.on_end(span(1));
    let flushed = queue.force_flush();

    flushed.unwrap();
    let names: Vec<_> = memory
        .exported_spans()
        .into_iter()
        .map(|span| span.name)
        .collect();
    assert_eq!(names, ["chat 0", "chat 1"]);
    assert_eq!(lost.take(), 0);
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn an_exporter_is_told_what_its_exports_come_from_before_it_exports() {
    let (memory, lost) = (Memory::default(), lost());

    let spans = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    let records = RecordQueue::new("memory", memory.records(), &resource(), &lost);

    assert_eq!(memory.resources(), [resource(), resource()]);
    assert!(memory.exports().is_empty());
    spans.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    records
        .shutdown_with_timeout(Duration::from_secs(5))
        .unwrap();
}

#[test]
fn a_span_there_is_no_room_for_is_counted_lost_and_the_rest_are_exported() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    memory.hold();

    for number in 0..QUEUE_CAPACITY + 3 {
        queue.on_end(span(number as u64));
    }
    let lost_while_held = lost.take();
    memory.release();
    let flushed = queue.force_flush();

    assert_eq!(lost_while_held, 3);
    flushed.unwrap();
    let exported = memory.exported_spans();
    assert_eq!(exported.len(), QUEUE_CAPACITY);
    assert_eq!(
        exported.last().unwrap().name,
        format!("chat {}", QUEUE_CAPACITY - 1),
        "the ones turned away are the ones that came last"
    );
    assert!(
        memory
            .exports()
            .iter()
            .all(|export| matches!(export, Export::Spans(spans) if spans.len() <= EXPORT_BATCH))
    );

    for number in 0..QUEUE_CAPACITY {
        queue.on_end(span(number as u64));
    }
    assert_eq!(
        lost.take(),
        0,
        "what was exported gave its room back, all of it"
    );
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(memory.exported_spans().len(), 2 * QUEUE_CAPACITY);
}

#[test]
fn the_spans_of_an_export_that_failed_are_counted_lost() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    memory.refuse_spans(true);

    for number in 0..3 {
        queue.on_end(span(number));
    }
    let flushed = queue.force_flush();

    assert!(flushed.is_err(), "{flushed:?}");
    assert_eq!(lost.take(), 3);
    assert!(memory.exported_spans().is_empty());
    let _ = queue.shutdown_with_timeout(Duration::from_secs(5));
}

// Log records

#[test]
fn a_queue_of_records_exports_what_it_holds_when_it_is_flushed() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = RecordQueue::new("memory", memory.records(), &resource(), &lost);

    emit(&queue, 2);
    let flushed = queue.force_flush();

    flushed.unwrap();
    assert_eq!(memory.exported_records().len(), 2);
    assert_eq!(lost.take(), 0);
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn a_record_there_is_no_room_for_is_counted_lost_and_the_rest_are_exported() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = RecordQueue::new("memory", memory.records(), &resource(), &lost);
    memory.hold();

    emit(&queue, QUEUE_CAPACITY + 2);
    let lost_while_held = lost.take();
    memory.release();
    let flushed = queue.force_flush();

    assert_eq!(lost_while_held, 2);
    flushed.unwrap();
    assert_eq!(memory.exported_records().len(), QUEUE_CAPACITY);

    emit(&queue, QUEUE_CAPACITY);
    assert_eq!(lost.take(), 0);
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(memory.exported_records().len(), 2 * QUEUE_CAPACITY);
}

#[test]
fn the_records_of_an_export_that_failed_are_counted_lost() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = RecordQueue::new("memory", memory.records(), &resource(), &lost);
    memory.refuse_records(true);

    emit(&queue, 4);
    let flushed = queue.force_flush();

    assert!(flushed.is_err(), "{flushed:?}");
    assert_eq!(lost.take(), 4);
    assert!(memory.exported_records().is_empty());
    let _ = queue.shutdown_with_timeout(Duration::from_secs(5));
}

#[test]
fn a_provider_has_nothing_to_tell_a_queue_of_its_resource() {
    let (memory, lost) = (Memory::default(), lost());
    let mut spans = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    let mut records = RecordQueue::new("memory", memory.records(), &resource(), &lost);

    spans.set_resource(&Resource::builder_empty().build());
    records.set_resource(&Resource::builder_empty().build());

    assert_eq!(memory.resources(), [resource(), resource()]);
    spans.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    records
        .shutdown_with_timeout(Duration::from_secs(5))
        .unwrap();
}

// Stopping a queue

#[test]
fn a_queue_is_the_same_queue_as_its_clone() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    let handed_to_a_provider = queue.clone();

    handed_to_a_provider.on_end(span(0));
    queue.force_flush().unwrap();

    assert_eq!(memory.exported_spans().len(), 1);
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        matches!(
            handed_to_a_provider.force_flush(),
            Err(OTelSdkError::AlreadyShutdown)
        ),
        "a stop through one handle stopped the queue the other holds"
    );
}

/// A provider stops its processors again when it's dropped, and the SDK's
/// processor waits for every stop it's asked for, so the second stop of a
/// queue whose destination doesn't answer would cost the provider's own
/// five seconds. The queue answers it at once instead.
#[test]
fn a_queue_that_was_told_to_stop_answers_every_later_stop_at_once() {
    let (memory, lost) = (Memory::default(), lost());
    let spans = SpanQueue::new("memory", memory.spans(), &resource(), &lost);
    let records = RecordQueue::new("memory", memory.records(), &resource(), &lost);
    memory.hold();
    spans.on_end(span(0));
    emit(&records, 1);
    let brief = Duration::from_millis(50);

    let first = (
        spans.shutdown_with_timeout(brief),
        records.shutdown_with_timeout(brief),
    );
    let began = Instant::now();
    let again = (
        spans.clone().shutdown_with_timeout(Duration::from_secs(5)),
        records
            .clone()
            .shutdown_with_timeout(Duration::from_secs(5)),
    );
    let waited = began.elapsed();
    memory.release();

    assert!(
        matches!(
            first,
            (
                Err(OTelSdkError::Timeout(spans)),
                Err(OTelSdkError::Timeout(records))
            ) if spans == brief && records == brief
        ),
        "the first stop waited for its timeout, since the destination held the export: {first:?}"
    );
    assert!(again.0.is_ok() && again.1.is_ok(), "{again:?}");
    assert!(
        waited < Duration::from_secs(1),
        "the second stop waited {waited:?}"
    );
}
