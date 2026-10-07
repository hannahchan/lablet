//! The SDK's own variables: the sampler and its argument, the span limits,
//! and the two batch processors' configs, each with the specification's
//! default where the environment gives none.
//!
//! Each is stated on the SDK whole. The SDK's builders read the same
//! variables into their defaults, case-sensitively and without a word for a
//! value they can't use, and what's stated here overwrites every value they
//! read.

use std::num::NonZeroUsize;
use std::time::Duration;

use opentelemetry_sdk::logs::{BatchConfig as LogBatchConfig, BatchConfigBuilder as LogBatch};
use opentelemetry_sdk::trace::{
    BatchConfig as SpanBatchConfig, BatchConfigBuilder as SpanBatch, Sampler, SpanLimits,
};

use super::{Choice, Named, Variables};

const TRACES_SAMPLER: &str = "OTEL_TRACES_SAMPLER";
const TRACES_SAMPLER_ARG: &str = "OTEL_TRACES_SAMPLER_ARG";

const SPAN_ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT";
const SPAN_EVENT_COUNT_LIMIT: &str = "OTEL_SPAN_EVENT_COUNT_LIMIT";
const SPAN_LINK_COUNT_LIMIT: &str = "OTEL_SPAN_LINK_COUNT_LIMIT";
const EVENT_ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT";
const LINK_ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_LINK_ATTRIBUTE_COUNT_LIMIT";
/// The limit beneath every attribute count of its own.
const ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_ATTRIBUTE_COUNT_LIMIT";

/// The specification's default of every count limit.
const LIMIT: u32 = 128;

/// The variables of one batch processor, and their defaults.
struct BatchVariables {
    schedule_delay: &'static str,
    max_queue_size: &'static str,
    max_export_batch_size: &'static str,
    default_schedule_delay: Duration,
}

const SPAN_BATCH: BatchVariables = BatchVariables {
    schedule_delay: "OTEL_BSP_SCHEDULE_DELAY",
    max_queue_size: "OTEL_BSP_MAX_QUEUE_SIZE",
    max_export_batch_size: "OTEL_BSP_MAX_EXPORT_BATCH_SIZE",
    default_schedule_delay: Duration::from_secs(5),
};

const LOG_BATCH: BatchVariables = BatchVariables {
    schedule_delay: "OTEL_BLRP_SCHEDULE_DELAY",
    max_queue_size: "OTEL_BLRP_MAX_QUEUE_SIZE",
    max_export_batch_size: "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
    default_schedule_delay: Duration::from_secs(1),
};

/// The specification's default queue size, for both processors.
const MAX_QUEUE_SIZE: usize = 2_048;
/// The specification's default batch size, for both processors.
const MAX_EXPORT_BATCH_SIZE: usize = 512;

/// The SDK's settings, as the environment gives them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sdk {
    /// Which traces are recorded.
    pub(crate) sampling: Sampling,
    /// How much a span keeps.
    pub(crate) limits: Limits,
    /// The span processor's batches.
    pub(crate) spans: Batch,
    /// The log record processor's batches.
    pub(crate) logs: Batch,
}

/// The specification's defaults, as an environment that sets nothing gives
/// them.
impl Default for Sdk {
    fn default() -> Self {
        Self::read(&Variables(&|_| None))
    }
}

impl Sdk {
    pub(super) fn read(variables: &Variables<'_>) -> Self {
        Self {
            sampling: Sampling::read(variables),
            limits: Limits::read(variables),
            spans: Batch::read(variables, &SPAN_BATCH),
            logs: Batch::read(variables, &LOG_BATCH),
        }
    }
}

/// A sampler the SDK builds, as `OTEL_TRACES_SAMPLER` names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sampling {
    /// Whether a span with a parent follows the parent's decision, and
    /// only a root span is sampled by `root`.
    pub(crate) parent_based: bool,
    /// How a span the parent doesn't decide for is sampled.
    pub(crate) root: Root,
}

/// How a span is sampled when no parent decides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Root {
    /// Every span is recorded.
    AlwaysOn,
    /// No span is recorded.
    AlwaysOff,
    /// This fraction of traces, from 0 to 1, by their ids.
    TraceIdRatio(f64),
}

/// The sampler names lablet builds, of the specification's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SamplerName {
    AlwaysOn,
    AlwaysOff,
    TraceIdRatio,
    ParentBasedAlwaysOn,
    ParentBasedAlwaysOff,
    ParentBasedTraceIdRatio,
}

impl Choice for SamplerName {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "always_on" => Self::AlwaysOn,
            "always_off" => Self::AlwaysOff,
            "traceidratio" => Self::TraceIdRatio,
            "parentbased_always_on" => Self::ParentBasedAlwaysOn,
            "parentbased_always_off" => Self::ParentBasedAlwaysOff,
            "parentbased_traceidratio" => Self::ParentBasedTraceIdRatio,
            _ => return None,
        })
    }
}

/// A ratio a sampler takes as its argument: a number from 0 to 1.
struct Ratio(f64);

impl super::Parse for Ratio {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        let ratio = text
            .parse::<f64>()
            .ok()
            .filter(|ratio| (0.0..=1.0).contains(ratio));
        if ratio.is_none() {
            ignored(
                text,
                "isn't a ratio from 0 to 1, so it's read as unset, which is 1",
            );
        }
        ratio.map(Self)
    }
}

impl Sampling {
    fn read(variables: &Variables<'_>) -> Self {
        let Some(Named(name)) = variables.get::<Named<SamplerName>>(TRACES_SAMPLER) else {
            return Self {
                parent_based: true,
                root: Root::AlwaysOn,
            };
        };
        // The argument is read only for a sampler that takes one, as the
        // specification uses it only then.
        let ratio = || {
            Root::TraceIdRatio(
                variables
                    .get::<Ratio>(TRACES_SAMPLER_ARG)
                    .map_or(1.0, |Ratio(ratio)| ratio),
            )
        };
        let (parent_based, root) = match name {
            SamplerName::AlwaysOn => (false, Root::AlwaysOn),
            SamplerName::AlwaysOff => (false, Root::AlwaysOff),
            SamplerName::TraceIdRatio => (false, ratio()),
            SamplerName::ParentBasedAlwaysOn => (true, Root::AlwaysOn),
            SamplerName::ParentBasedAlwaysOff => (true, Root::AlwaysOff),
            SamplerName::ParentBasedTraceIdRatio => (true, ratio()),
        };
        Self { parent_based, root }
    }

    /// The SDK's sampler.
    pub(crate) fn sampler(self) -> Sampler {
        let root = match self.root {
            Root::AlwaysOn => Sampler::AlwaysOn,
            Root::AlwaysOff => Sampler::AlwaysOff,
            Root::TraceIdRatio(ratio) => Sampler::TraceIdRatioBased(ratio),
        };
        if self.parent_based {
            Sampler::ParentBased(Box::new(root))
        } else {
            root
        }
    }
}

/// How much a span keeps, as the five span limits say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The attributes of a span.
    pub(crate) attributes_per_span: u32,
    /// The events of a span.
    pub(crate) events_per_span: u32,
    /// The links of a span.
    pub(crate) links_per_span: u32,
    /// The attributes of one of a span's events.
    pub(crate) attributes_per_event: u32,
    /// The attributes of one of a span's links.
    pub(crate) attributes_per_link: u32,
}

impl Limits {
    fn read(variables: &Variables<'_>) -> Self {
        let attributes = variables.get::<u32>(ATTRIBUTE_COUNT_LIMIT);
        let count = |name: &str| variables.get::<u32>(name);
        let attribute_count = |name: &str| count(name).or(attributes).unwrap_or(LIMIT);
        Self {
            attributes_per_span: attribute_count(SPAN_ATTRIBUTE_COUNT_LIMIT),
            events_per_span: count(SPAN_EVENT_COUNT_LIMIT).unwrap_or(LIMIT),
            links_per_span: count(SPAN_LINK_COUNT_LIMIT).unwrap_or(LIMIT),
            attributes_per_event: attribute_count(EVENT_ATTRIBUTE_COUNT_LIMIT),
            attributes_per_link: attribute_count(LINK_ATTRIBUTE_COUNT_LIMIT),
        }
    }

    /// The SDK's span limits. Written whole, with no field left to its
    /// default, so a limit the SDK adds is a field this fails to build
    /// without, rather than one the SDK fills from the process environment.
    pub(crate) fn span_limits(self) -> SpanLimits {
        SpanLimits {
            max_events_per_span: self.events_per_span,
            max_attributes_per_span: self.attributes_per_span,
            max_links_per_span: self.links_per_span,
            max_attributes_per_event: self.attributes_per_event,
            max_attributes_per_link: self.attributes_per_link,
        }
    }
}

/// How one batch processor batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Batch {
    /// How long the processor waits between two exports.
    pub(crate) schedule_delay: Duration,
    /// How many spans or records it holds before it drops the next.
    pub(crate) max_queue_size: usize,
    /// How many it exports at once at most.
    pub(crate) max_export_batch_size: usize,
}

impl Batch {
    fn read(variables: &Variables<'_>, names: &BatchVariables) -> Self {
        Self {
            schedule_delay: variables
                .get(names.schedule_delay)
                .unwrap_or(names.default_schedule_delay),
            max_queue_size: variables
                .get(names.max_queue_size)
                .map_or(MAX_QUEUE_SIZE, NonZeroUsize::get),
            max_export_batch_size: variables
                .get(names.max_export_batch_size)
                .map_or(MAX_EXPORT_BATCH_SIZE, NonZeroUsize::get),
        }
    }

    /// The span processor's config, with every field the thread-based
    /// processor reads stated.
    pub(crate) fn span_config(self) -> SpanBatchConfig {
        SpanBatch::default()
            .with_scheduled_delay(self.schedule_delay)
            .with_max_queue_size(self.max_queue_size)
            .with_max_export_batch_size(self.max_export_batch_size)
            .build()
    }

    /// The log record processor's config, with every field the thread-based
    /// processor reads stated.
    pub(crate) fn log_config(self) -> LogBatchConfig {
        LogBatch::default()
            .with_scheduled_delay(self.schedule_delay)
            .with_max_queue_size(self.max_queue_size)
            .with_max_export_batch_size(self.max_export_batch_size)
            .build()
    }
}

#[cfg(test)]
mod tests;
