use std::time::UNIX_EPOCH;

use opentelemetry::KeyValue;
use opentelemetry::logs::{LoggerProvider as _, Severity};
use opentelemetry::trace::{SpanId, SpanKind, TraceId};
use opentelemetry_sdk::logs::SdkLoggerProvider;

use super::*;
use crate::attributes::Attributes;
use crate::signal::{Ended, Record, Span};
use crate::testing::memory::Memory;

const TRACE: TraceId = TraceId::from_bytes([0xab; 16]);

fn resource() -> Resource {
    Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "lablet"))
        .build()
}

fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet").build()
}

fn span(number: u64) -> SpanData {
    Span {
        name: format!("chat {number}"),
        kind: SpanKind::Client,
        id: SpanId::from(number + 1),
        parent: None,
        start: UNIX_EPOCH,
        end: UNIX_EPOCH,
        attributes: Attributes::default(),
        events: Vec::new(),
        ended: Ended::Well,
    }
    .into_data(TRACE, &scope())
}

/// Emits `records` log records to `queue`, as a logger provider does.
fn emit(queue: &RecordQueue, records: usize) {
    let provider = SdkLoggerProvider::builder().build();
    let logger = provider.logger_with_scope(scope());
    for _ in 0..records {
        let mut record = Record {
            name: "lablet.test",
            severity: Severity::Info,
            at: UNIX_EPOCH,
            span: SpanId::from(1),
            attributes: Attributes::default(),
        }
        .into_sdk(TRACE, &logger);
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

    room.left(1, true);

    assert_eq!([room.admit(), room.admit()], [true, false]);
    room.left(2, true);
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

    room.left(2, false);

    assert_eq!(lost.take(), 2);
    assert_eq!(
        [room.admit(), room.admit(), room.admit()],
        [true, true, false]
    );
}

#[test]
fn a_count_that_was_taken_starts_again_from_none() {
    let lost = lost();
    lost.add(3);
    lost.add(4);

    assert_eq!(lost.take(), 7);
    assert_eq!(lost.take(), 0);
    lost.add(1);
    assert_eq!(lost.take(), 1);
}

// Spans

#[test]
fn a_queue_of_spans_exports_what_it_holds_when_it_is_flushed() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new(memory.spans(), &resource(), &lost);

    queue.end(span(0));
    queue.end(span(1));
    let flushed = queue.flush();

    flushed.unwrap();
    let names: Vec<_> = memory
        .exported_spans()
        .into_iter()
        .map(|span| span.name)
        .collect();
    assert_eq!(names, ["chat 0", "chat 1"]);
    assert_eq!(lost.take(), 0);
    queue.shutdown(Duration::from_secs(5)).unwrap();
}

#[test]
fn an_exporter_is_told_what_its_exports_come_from_before_it_exports() {
    let (memory, lost) = (Memory::default(), lost());

    let spans = SpanQueue::new(memory.spans(), &resource(), &lost);
    let records = RecordQueue::new(memory.records(), &resource(), &lost);

    assert_eq!(memory.resources(), [resource(), resource()]);
    assert!(memory.exports().is_empty());
    spans.shutdown(Duration::from_secs(5)).unwrap();
    records
        .shutdown_with_timeout(Duration::from_secs(5))
        .unwrap();
}

#[test]
fn a_span_there_is_no_room_for_is_counted_lost_and_the_rest_are_exported() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new(memory.spans(), &resource(), &lost);
    memory.hold();

    for number in 0..QUEUE_CAPACITY + 3 {
        queue.end(span(number as u64));
    }
    let lost_while_held = lost.take();
    memory.release();
    let flushed = queue.flush();

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
            .all(|export| matches!(export, crate::testing::memory::Export::Spans(spans) if spans.len() <= EXPORT_BATCH))
    );

    for number in 0..QUEUE_CAPACITY {
        queue.end(span(number as u64));
    }
    assert_eq!(
        lost.take(),
        0,
        "what was exported gave its room back, all of it"
    );
    queue.shutdown(Duration::from_secs(5)).unwrap();
    assert_eq!(memory.exported_spans().len(), 2 * QUEUE_CAPACITY);
}

#[test]
fn the_spans_of_an_export_that_failed_are_counted_lost() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = SpanQueue::new(memory.spans(), &resource(), &lost);
    memory.refuse_spans(true);

    for number in 0..3 {
        queue.end(span(number));
    }
    let flushed = queue.flush();

    assert!(flushed.is_err(), "{flushed:?}");
    assert_eq!(lost.take(), 3);
    assert!(memory.exported_spans().is_empty());
    let _ = queue.shutdown(Duration::from_secs(5));
}

// Log records

#[test]
fn a_queue_of_records_exports_what_it_holds_when_it_is_flushed() {
    let (memory, lost) = (Memory::default(), lost());
    let queue = RecordQueue::new(memory.records(), &resource(), &lost);

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
    let queue = RecordQueue::new(memory.records(), &resource(), &lost);
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
    let queue = RecordQueue::new(memory.records(), &resource(), &lost);
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
    let mut queue = RecordQueue::new(memory.records(), &resource(), &lost);

    queue.set_resource(&Resource::builder_empty().build());

    assert_eq!(memory.resources(), [resource()]);
    queue.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
}
