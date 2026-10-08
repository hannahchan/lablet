//! The resource's and the inbound context's share of the seam: the
//! service's name, the resource's attributes, the propagators, and the
//! variables those propagators extract a parent from.

use std::collections::BTreeSet;

use lablet_env_carrier::variable;
use opentelemetry::Context as OtelContext;
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{
    Extractor, Injector, TextMapCompositePropagator, TextMapPropagator,
};
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry_propagator_b3::{B3Encoding, Propagator as B3};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};

use super::{Choice, Parse, Variables};

const SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
const RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";
const PROPAGATORS: &str = "OTEL_PROPAGATORS";

/// The resource's and the inbound context's variables, as the seam reads
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// What `OTEL_SERVICE_NAME` names the service, when it's set.
    pub(crate) service_name: Option<String>,
    /// The pairs of `OTEL_RESOURCE_ATTRIBUTES`, each key and value
    /// percent-decoded, in the order it holds them.
    pub(crate) resource_attributes: Vec<(String, String)>,
    /// The propagators, in the order `OTEL_PROPAGATORS` names them; none
    /// when it's `none`.
    pub(crate) propagators: Vec<Propagator>,
    /// What the variable of each key the propagators name holds, by the
    /// variable's name, for those that are set, in the order of their
    /// names.
    pub(crate) carried: Vec<(String, String)>,
}

/// What an environment that sets nothing gives.
impl Default for Context {
    fn default() -> Self {
        Self::read(&Variables(&|_| None))
    }
}

impl Context {
    pub(super) fn read(variables: &Variables<'_>) -> Self {
        let propagators = variables
            .get(PROPAGATORS)
            .unwrap_or_else(|| vec![Propagator::TraceContext, Propagator::Baggage]);
        let names: BTreeSet<String> = composite(&propagators).fields().map(variable).collect();
        let carried = names
            .into_iter()
            .filter_map(|name| variables.get(&name).map(|value| (name, value)))
            .collect();
        Self {
            service_name: variables.get(SERVICE_NAME),
            resource_attributes: variables
                .get(RESOURCE_ATTRIBUTES)
                .map(|ResourceAttributes(pairs)| pairs)
                .unwrap_or_default(),
            propagators,
            carried,
        }
    }

    /// The composite of the propagators, in their order: lablet's own,
    /// never the global one.
    pub(crate) fn propagator(&self) -> TextMapCompositePropagator {
        composite(&self.propagators)
    }
}

/// The composite of `propagators`, in their order.
fn composite(propagators: &[Propagator]) -> TextMapCompositePropagator {
    TextMapCompositePropagator::new(
        propagators
            .iter()
            .map(|propagator| -> Box<dyn TextMapPropagator + Send + Sync> {
                match propagator {
                    Propagator::TraceContext => Box::new(TraceContextPropagator::new()),
                    Propagator::Baggage => Box::new(BaggagePropagator::new()),
                    // The specification's `b3` injects the single header,
                    // which isn't the crate's default.
                    Propagator::B3 => {
                        Box::new(AsSpecified(B3::with_encoding(B3Encoding::SingleHeader)))
                    }
                    Propagator::B3Multi => {
                        Box::new(AsSpecified(B3::with_encoding(B3Encoding::MultipleHeader)))
                    }
                }
            })
            .collect(),
    )
}

/// `P`, injecting nothing for a context with no valid span, and only the
/// keys its fields name for one with. For a context with no valid span, as
/// under `OTEL_SDK_DISABLED=true` with no inbound context, the B3 crate
/// injects a sampled flag of its own, which a child's tracer reads as a
/// decision, though none was made. Its single header encoding injects the
/// multiple headers too, which the specification's `b3` doesn't. A canary
/// pins each.
#[derive(Debug)]
struct AsSpecified<P>(P);

impl<P: TextMapPropagator> TextMapPropagator for AsSpecified<P> {
    fn inject_context(&self, cx: &OtelContext, injector: &mut dyn Injector) {
        if !cx.span().span_context().is_valid() {
            return;
        }
        let fields = self.0.fields().collect();
        self.0.inject_context(cx, &mut Fielded { fields, injector });
    }

    fn extract_with_context(&self, cx: &OtelContext, extractor: &dyn Extractor) -> OtelContext {
        self.0.extract_with_context(cx, extractor)
    }

    fn fields(&self) -> FieldIter<'_> {
        self.0.fields()
    }
}

/// An injector that sets the keys of `fields` and drops the rest.
struct Fielded<'a> {
    fields: Vec<&'a str>,
    injector: &'a mut dyn Injector,
}

impl Injector for Fielded<'_> {
    fn set(&mut self, key: &str, value: String) {
        if self.fields.contains(&key) {
            self.injector.set(key, value);
        }
    }
}

/// A propagator lablet serves, of those the specification names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Propagator {
    /// The W3C trace context: `traceparent` and `tracestate`.
    TraceContext,
    /// The W3C baggage: `baggage`.
    Baggage,
    /// B3's single header: `b3`.
    B3,
    /// B3's multiple headers: `x-b3-traceid`, `x-b3-spanid`, `x-b3-sampled`
    /// and `x-b3-flags`.
    B3Multi,
}

impl Choice for Propagator {
    fn named(name: &str) -> Option<Self> {
        match name {
            "tracecontext" => Some(Self::TraceContext),
            "baggage" => Some(Self::Baggage),
            "b3" => Some(Self::B3),
            "b3multi" => Some(Self::B3Multi),
            _ => None,
        }
    }
}

/// The pairs of `OTEL_RESOURCE_ATTRIBUTES`: `key=value`, comma-separated,
/// each side trimmed and then percent-decoded. A blank member, as a
/// trailing comma leaves, is skipped. Any other error, a pair with no `=`
/// or no key or a part that doesn't decode, discards the whole value, as
/// the resource specification asks, so no half of it is used.
struct ResourceAttributes(Vec<(String, String)>);

impl Parse for ResourceAttributes {
    /// A framework may put what it keeps to itself in the resource, and a
    /// value that doesn't decode can't be shown as it would read.
    const SECRET: bool = true;

    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        let mut pairs = Vec::new();
        for pair in text.split(',').filter(|pair| !pair.trim().is_empty()) {
            let Some((key, value)) = pair.split_once('=') else {
                ignored(
                    text,
                    "has a pair with no `=`, so the whole of it is ignored",
                );
                return None;
            };
            let (Some(key), Some(value)) = (decoded(key.trim()), decoded(value.trim())) else {
                ignored(
                    text,
                    "doesn't percent-decode, so the whole of it is ignored",
                );
                return None;
            };
            if key.is_empty() {
                ignored(
                    text,
                    "has a pair with no key, so the whole of it is ignored",
                );
                return None;
            }
            pairs.push((key, value));
        }
        Some(Self(pairs))
    }
}

/// `text` with each `%` and the two hex digits after it read as the byte
/// they spell, or nothing when a `%` has no two digits after it or the
/// bytes aren't UTF-8.
fn decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = hex_digit(rest.next()?)?;
            let low = hex_digit(rest.next()?)?;
            bytes.push((high << 4) | low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
