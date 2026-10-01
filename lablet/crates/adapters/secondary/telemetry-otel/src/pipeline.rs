//! The way from the observer to an exporter: the SDK's batch processors,
//! which keep export off the loop's path, behind a count of what they hold,
//! so that nothing is lost without being counted.
//!
//! A batch processor drops what it's handed when its queue is full, and
//! says so only to its own log. So the observer keeps the count of what each
//! queue holds and turns a span or a record away itself when there's no room
//! for it, which a processor whose queue is as long then never has to.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{
    BatchConfigBuilder as LogBatchConfig, BatchLogProcessor, LogBatch, LogExporter, LogProcessor,
    SdkLogRecord,
};
use opentelemetry_sdk::trace::{
    BatchConfigBuilder as SpanBatchConfig, BatchSpanProcessor, SpanData, SpanExporter,
    SpanProcessor as _,
};

/// How many spans or records one queue holds before the observer turns the
/// next away.
pub(crate) const QUEUE_CAPACITY: usize = 2_048;

/// How many spans or records one export holds at most.
pub(crate) const EXPORT_BATCH: usize = 512;

/// How long a processor lets what it holds wait for a batch to fill.
///
/// No export timeout is set beside it: the SDK's thread-based processors
/// wait on an export for as long as it takes, and only their experimental
/// async-runtime processors have the setter. The exporter's own timeout is
/// what bounds one export, and the observer's flush bound is what bounds
/// the wait.
const EXPORT_EVERY: Duration = Duration::from_secs(1);

/// The count of what one destination's exporters lost, by generation.
#[derive(Debug, Default)]
struct Count {
    /// Which run's count this is: the wide event of a run takes the count
    /// and starts the next generation.
    generation: u64,
    lost: u64,
}

/// How many spans and records one destination's exporters have lost: the
/// ones turned away from a full queue, and the ones of an export that
/// failed.
///
/// The count is of one generation at a time, which ends when it's taken for
/// a run's wide event. An export that was in flight when the flush gave up
/// waiting for it fails later; its loss is of the generation it began in,
/// which has ended, so it's counted against no run and logged instead.
#[derive(Debug, Default)]
pub(crate) struct Lost(Mutex<Count>);

impl Lost {
    fn count(&self) -> std::sync::MutexGuard<'_, Count> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The generation now: the one whose count the next [`Lost::take`]
    /// gives.
    pub(crate) fn generation(&self) -> u64 {
        self.count().generation
    }

    /// Counts `records` lost in `generation`.
    fn add(&self, generation: u64, records: usize) {
        let mut count = self.count();
        if count.generation == generation {
            count.lost += records as u64;
        } else {
            drop(count);
            tracing::warn!(
                records,
                "records were lost after the wide event of their run was made, so no run counts them"
            );
        }
    }

    /// The count of the generation now, which then ends: what's lost from
    /// here on is the next generation's.
    pub(crate) fn take(&self) -> u64 {
        let mut count = self.count();
        count.generation += 1;
        std::mem::take(&mut count.lost)
    }
}

/// The room in one queue.
#[derive(Debug)]
pub(crate) struct Room {
    capacity: usize,
    held: AtomicUsize,
    lost: Arc<Lost>,
}

impl Room {
    pub(crate) fn new(capacity: usize, lost: Arc<Lost>) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            held: AtomicUsize::new(0),
            lost,
        })
    }

    /// Takes the room of one span or record, or counts it lost when the
    /// queue has none.
    fn admit(&self) -> bool {
        let admitted = self
            .held
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < self.capacity).then_some(held + 1)
            })
            .is_ok();
        if !admitted {
            self.lost.add(self.lost.generation(), 1);
        }
        admitted
    }

    /// The generation an export that begins now is of.
    fn generation(&self) -> u64 {
        self.lost.generation()
    }

    /// Gives back the room of `records` that left the queue in one export
    /// that began in `generation`, and counts them lost when the export
    /// failed.
    fn left(&self, records: usize, exported: bool, generation: u64) {
        self.held.fetch_sub(records, Ordering::AcqRel);
        if !exported {
            self.lost.add(generation, records);
        }
    }
}

/// An exporter that says what became of each batch it was handed.
///
/// A failed export goes to the diagnostic log as well as to the count: it
/// changes nothing about the run, and someone has to be told.
#[derive(Debug)]
struct Counted<E> {
    /// The destination, as the diagnostic log names it.
    destination: &'static str,
    exporter: E,
    room: Arc<Room>,
}

impl<E: SpanExporter> SpanExporter for Counted<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let spans = batch.len();
        let generation = self.room.generation();
        let result = self.exporter.export(batch).await;
        self.room.left(spans, result.is_ok(), generation);
        if let Err(error) = &result {
            tracing::warn!(
                %error,
                spans,
                destination = self.destination,
                "an export of spans failed, and the spans are lost"
            );
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.exporter.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.exporter.set_resource(resource);
    }
}

impl<E: LogExporter> LogExporter for Counted<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let records = batch.iter().count();
        let generation = self.room.generation();
        let result = self.exporter.export(batch).await;
        self.room.left(records, result.is_ok(), generation);
        if let Err(error) = &result {
            tracing::warn!(
                %error,
                records,
                destination = self.destination,
                "an export of log records failed, and the records are lost"
            );
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.exporter.set_resource(resource);
    }
}

/// The queue of spans on their way to one exporter.
#[derive(Debug)]
pub(crate) struct SpanQueue {
    processor: BatchSpanProcessor,
    room: Arc<Room>,
}

impl SpanQueue {
    /// A queue to `exporter`, which describes what it exports as coming
    /// from `resource`, at the destination `destination` names.
    pub(crate) fn new(
        destination: &'static str,
        exporter: impl SpanExporter + 'static,
        resource: &Resource,
        lost: &Arc<Lost>,
    ) -> Self {
        let room = Room::new(QUEUE_CAPACITY, Arc::clone(lost));
        let mut exporter = Counted {
            destination,
            exporter,
            room: Arc::clone(&room),
        };
        exporter.set_resource(resource);
        let processor = BatchSpanProcessor::builder(exporter)
            .with_batch_config(
                SpanBatchConfig::default()
                    .with_max_queue_size(QUEUE_CAPACITY)
                    .with_max_export_batch_size(EXPORT_BATCH)
                    .with_scheduled_delay(EXPORT_EVERY)
                    .build(),
            )
            .build();
        Self { processor, room }
    }

    /// Hands over a span that has ended.
    pub(crate) fn end(&self, span: SpanData) {
        if self.room.admit() {
            self.processor.on_end(span);
        }
    }

    pub(crate) fn flush(&self) -> OTelSdkResult {
        self.processor.force_flush()
    }

    pub(crate) fn shutdown(&self, timeout: Duration) -> OTelSdkResult {
        self.processor.shutdown_with_timeout(timeout)
    }
}

/// The queue of log records on their way to one exporter, as a processor
/// of a logger provider.
#[derive(Debug)]
pub(crate) struct RecordQueue {
    processor: BatchLogProcessor,
    room: Arc<Room>,
}

impl RecordQueue {
    /// A queue to `exporter`, which describes what it exports as coming
    /// from `resource`, at the destination `destination` names.
    pub(crate) fn new(
        destination: &'static str,
        exporter: impl LogExporter + 'static,
        resource: &Resource,
        lost: &Arc<Lost>,
    ) -> Self {
        let room = Room::new(QUEUE_CAPACITY, Arc::clone(lost));
        let mut exporter = Counted {
            destination,
            exporter,
            room: Arc::clone(&room),
        };
        exporter.set_resource(resource);
        let processor = BatchLogProcessor::builder(exporter)
            .with_batch_config(
                LogBatchConfig::default()
                    .with_max_queue_size(QUEUE_CAPACITY)
                    .with_max_export_batch_size(EXPORT_BATCH)
                    .with_scheduled_delay(EXPORT_EVERY)
                    .build(),
            )
            .build();
        Self { processor, room }
    }
}

impl LogProcessor for RecordQueue {
    fn emit(&self, record: &mut SdkLogRecord, scope: &InstrumentationScope) {
        if self.room.admit() {
            self.processor.emit(record, scope);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.processor.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.processor.shutdown_with_timeout(timeout)
    }

    /// The exporter was given its resource when the queue was made, ahead
    /// of anything it could export, so a provider has nothing to add.
    fn set_resource(&mut self, _resource: &Resource) {}
}

#[cfg(test)]
mod tests;
