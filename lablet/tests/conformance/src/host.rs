//! A host's OpenTelemetry, as a test of the library hands it in: the SDK's
//! tracer and logger providers over its in-memory exporters, each span and
//! record exported as it ends or is emitted, and what they hold read back
//! as the file exporter's lines would read, so a test of the library
//! asserts on a run's spans and records as a test of the command line does.

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::{InMemoryLogExporter, LogBatch, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use crate::must;
use crate::otlp::Exported;

/// The service a host's resource names, which no export of lablet's own
/// names.
pub const SERVICE: &str = "a-host";

/// A host's providers and what they've been handed.
#[derive(Debug, Clone)]
pub struct Host {
    resource: Resource,
    spans: InMemorySpanExporter,
    records: InMemoryLogExporter,
    tracer_provider: SdkTracerProvider,
    logger_provider: SdkLoggerProvider,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    /// A host whose resource names [`SERVICE`] and nothing else.
    #[must_use]
    pub fn new() -> Self {
        let resource = Resource::builder_empty().with_service_name(SERVICE).build();
        let spans = InMemorySpanExporter::default();
        let records = InMemoryLogExporter::default();
        Self {
            tracer_provider: SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_simple_exporter(spans.clone())
                .build(),
            logger_provider: SdkLoggerProvider::builder()
                .with_resource(resource.clone())
                .with_simple_exporter(records.clone())
                .build(),
            resource,
            spans,
            records,
        }
    }

    /// The tracer provider, to hand in.
    #[must_use]
    pub fn tracer_provider(&self) -> SdkTracerProvider {
        self.tracer_provider.clone()
    }

    /// The logger provider, to hand in.
    #[must_use]
    pub fn logger_provider(&self) -> SdkLoggerProvider {
        self.logger_provider.clone()
    }

    /// What the providers were handed so far, read back.
    ///
    /// # Panics
    ///
    /// When an exporter can't be read, or what it holds doesn't read back,
    /// which is a fault of the test's own.
    #[must_use]
    pub fn exported(&self) -> Exported {
        must(Exported::parse(&self.lines()), "reading the lines back")
    }

    /// What the providers were handed so far, as the file exporter's lines:
    /// a line for each span in the order they ended, and then a line for
    /// each record in the order they were emitted.
    ///
    /// # Panics
    ///
    /// When an exporter can't be read, which is a fault of the test's own.
    #[must_use]
    pub fn lines(&self) -> String {
        let resource = ResourceAttributesWithSchema::from(&self.resource);
        let mut lines = String::new();
        for span in must(self.spans.get_finished_spans(), "reading the spans") {
            let request = ExportTraceServiceRequest {
                resource_spans: group_spans_by_resource_and_scope(vec![span], &resource),
            };
            lines.push_str(&must(serde_json::to_string(&request), "writing a span"));
            lines.push('\n');
        }
        for log in must(self.records.get_emitted_logs(), "reading the records") {
            let one = [(&log.record, &log.instrumentation)];
            let request = ExportLogsServiceRequest {
                resource_logs: group_logs_by_resource_and_scope(&LogBatch::new(&one), &resource),
            };
            lines.push_str(&must(serde_json::to_string(&request), "writing a record"));
            lines.push('\n');
        }
        lines
    }
}
