//! The `telemetry` section: what's captured, and where it's exported.
//!
//! A field that's `null` isn't stated, and nothing else fills it in: a run
//! sends and writes nothing until the config names a place, and no variable
//! is read but through a `${VAR}` the config writes. Three settings take two
//! forms, since a platform injects them as OpenTelemetry's text: the
//! propagators as a list or a comma text, and the headers and the resource
//! as a map or `k=v` text.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use lablet_otel_env::decode_headers;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap as _;
use serde::{Deserialize, Serialize, Serializer};

use super::secret::Secret;
use super::written::path;

/// The `telemetry` section. A field that's `null` isn't stated: a run sends
/// and writes nothing until the config names a place, and no variable is
/// read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    /// Whether prompts, responses and tool content reach telemetry. `None`
    /// captures nothing, as `false` does.
    pub capture_content: Option<bool>,
    /// The propagators the inbound context is read through and a command's
    /// context is injected through: a list of names, or OpenTelemetry's
    /// comma text, of `tracecontext`, `baggage`, `b3` and `b3multi`, or
    /// `none` alone. `None` is `tracecontext,baggage`.
    pub propagators: Option<Propagators>,
    /// The OTLP network exporter.
    pub otlp: Otlp,
    /// The OTLP/JSON file exporter.
    pub file: TelemetryFile,
    /// The resource attributes every export carries: a map, or
    /// OpenTelemetry's `k=v` text with each value percent-decoded. The first
    /// `service.name` names the service, and `lablet` when none does;
    /// `service.version` and `telemetry.sdk.*` stay lablet's and the SDK's.
    pub resource: Resource,
}

/// The `telemetry.otlp` section. With `endpoint` null nothing goes over the
/// network, whatever else is stated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Otlp {
    /// Where the collector listens: a base URL, to which `/v1/traces` and
    /// `/v1/logs` are appended on HTTP, and both signals go there. `None`
    /// exports nothing over the network. Its user information is a secret,
    /// cut everywhere and out of the digest.
    pub endpoint: Option<String>,
    /// The protocol the collector is sent; `None` is HTTP/protobuf.
    pub protocol: Option<OtlpProtocol>,
    /// Headers sent with every export, and the only ones: a map, or
    /// OpenTelemetry's `k=v` text with each value percent-decoded. Every
    /// value is lablet's own and holds a `${VAR}`: a variable filled into one
    /// reaches no command, the value is cut out of every tool result, and
    /// it's left out of the config digest.
    pub headers: Option<Headers>,
    /// TLS to the collector.
    pub tls: Tls,
}

/// The `telemetry.otlp.tls` section: PEM files, each a path a `${VAR}` may
/// fill, read whenever an endpoint is named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Tls {
    /// The certificates the exporter trusts, in place of the platform's
    /// roots, for this exporter alone.
    #[serde(serialize_with = "path::optional")]
    pub ca_file: Option<PathBuf>,
    /// The exporter's own certificate, for a collector that asks for one;
    /// it lands together with `client_key`.
    #[serde(serialize_with = "path::optional")]
    pub client_cert: Option<PathBuf>,
    /// The private key of `client_cert`, whose contents are cut out of every
    /// tool result.
    #[serde(serialize_with = "path::optional")]
    pub client_key: Option<PathBuf>,
}

/// The protocol an OTLP collector is sent, as the specification spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
pub enum OtlpProtocol {
    /// OTLP over gRPC.
    #[serde(rename = "grpc")]
    Grpc,
    /// OTLP over HTTP, as protobuf.
    #[serde(rename = "http/protobuf")]
    HttpProtobuf,
    /// OTLP over HTTP, as JSON.
    #[serde(rename = "http/json")]
    HttpJson,
}

/// The `telemetry.file` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryFile {
    /// The file every run's OTLP/JSON lines are appended to, or `-` for
    /// standard error. `None` writes no file, and the file gets both
    /// signals whatever the network does.
    #[serde(serialize_with = "path::optional")]
    pub path: Option<PathBuf>,
}

/// A propagator lablet serves, of those the specification names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Propagator {
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

impl Propagator {
    /// Every propagator, in the order the specification lists them.
    pub const ALL: [Self; 4] = [Self::TraceContext, Self::Baggage, Self::B3, Self::B3Multi];

    /// The name the config writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TraceContext => "tracecontext",
            Self::Baggage => "baggage",
            Self::B3 => "b3",
            Self::B3Multi => "b3multi",
        }
    }

    /// The propagator `name` spells, in any case and trimmed, when it's one
    /// lablet serves.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|propagator| propagator.as_str() == name)
    }
}

/// What a propagators setting accepts, as a refusal lists it.
const PROPAGATOR_NAMES: &str = "`tracecontext`, `baggage`, `b3`, `b3multi` and `none`";

/// The propagators `telemetry.propagators` names, in their order, each once;
/// none for `none`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Propagators(Vec<Propagator>);

impl Propagators {
    /// The specification's default, `tracecontext,baggage`, which a config
    /// that states nothing gets.
    #[must_use]
    pub fn specified() -> Self {
        Self(vec![Propagator::TraceContext, Propagator::Baggage])
    }

    /// No propagator, as `none` states.
    #[must_use]
    pub const fn none() -> Self {
        Self(Vec::new())
    }

    /// The propagators, in their order.
    #[must_use]
    pub fn names(&self) -> &[Propagator] {
        &self.0
    }

    /// The propagators the comma text `text` names, as OpenTelemetry's
    /// `OTEL_PROPAGATORS` is written: each name trimmed and matched in any
    /// case, a name written twice counted once, and `none` alone naming no
    /// propagator.
    ///
    /// # Errors
    ///
    /// Returns why the text is refused: a name lablet doesn't serve, or
    /// `none` beside another name.
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::of_names(text.split(','))
    }

    fn of_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let names: Vec<&str> = names
            .into_iter()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        if names.iter().any(|name| name.eq_ignore_ascii_case("none")) {
            return if names.len() == 1 {
                Ok(Self::none())
            } else {
                Err("`none` stands alone, and names no propagator".to_owned())
            };
        }
        let mut chosen = Vec::new();
        for name in names {
            let Some(propagator) = Propagator::named(name) else {
                return Err(format!("the accepted values are {PROPAGATOR_NAMES}"));
            };
            if !chosen.contains(&propagator) {
                chosen.push(propagator);
            }
        }
        Ok(Self(chosen))
    }
}

impl From<Vec<Propagator>> for Propagators {
    fn from(names: Vec<Propagator>) -> Self {
        Self::of_names(names.iter().map(|name| name.as_str())).unwrap_or_default()
    }
}

impl Default for Propagators {
    fn default() -> Self {
        Self::specified()
    }
}

impl<'de> Deserialize<'de> for Propagators {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NamesVisitor;

        impl<'de> Visitor<'de> for NamesVisitor {
            type Value = Propagators;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "a list of propagators, or a comma text, of {PROPAGATOR_NAMES}"
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Propagators, E> {
                Propagators::parse(text).map_err(E::custom)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Propagators, A::Error> {
                let mut names: Vec<String> = Vec::new();
                while let Some(name) = items.next_element::<String>()? {
                    names.push(name);
                }
                Propagators::of_names(names.iter().map(String::as_str)).map_err(A::Error::custom)
            }
        }

        deserializer.deserialize_any(NamesVisitor)
    }
}

/// Written as a list of names, `[]` for `none`, whichever form was read.
impl Serialize for Propagators {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|name| name.as_str()))
    }
}

impl JsonSchema for Propagators {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Propagators".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let names = generator.subschema_for::<Vec<Propagator>>().to_value();
        json_schema!({
            "anyOf": [
                {
                    "type": "string",
                    "description": "OpenTelemetry's comma text, as `tracecontext,baggage`; `none` alone names no propagator"
                },
                names
            ]
        })
    }
}

/// The headers `telemetry.otlp.headers` states, each value a credential:
/// a map of names to values, or OpenTelemetry's `k=v` text, whose pairs are
/// decoded as the exporter decodes `OTEL_EXPORTER_OTLP_HEADERS`, each value
/// percent-decoded and a pair with an empty name or value left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headers {
    pairs: BTreeMap<String, Secret>,
    /// The text, when the config wrote one, which is a secret whole.
    text: Option<Secret>,
}

impl Headers {
    /// The headers the `k=v` text `text` names.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let pairs = decode_headers(text)
            .into_iter()
            .map(|(name, value)| (name, Secret::new(value)))
            .collect();
        Self {
            pairs,
            text: Some(Secret::new(text)),
        }
    }

    /// Each header, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Secret)> {
        self.pairs.iter()
    }

    /// The value of the header `name`, when there's one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Secret> {
        self.pairs.get(name)
    }

    /// Whether no header is named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// How many headers are named.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// The `k=v` text the config wrote, when it wrote one rather than a
    /// map.
    #[must_use]
    pub fn text(&self) -> Option<&Secret> {
        self.text.as_ref()
    }
}

impl From<BTreeMap<String, Secret>> for Headers {
    fn from(pairs: BTreeMap<String, Secret>) -> Self {
        Self { pairs, text: None }
    }
}

impl<K: Into<String>> FromIterator<(K, Secret)> for Headers {
    fn from_iter<I: IntoIterator<Item = (K, Secret)>>(pairs: I) -> Self {
        Self {
            pairs: pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
            text: None,
        }
    }
}

impl<K: Into<String>, const N: usize> From<[(K, Secret); N]> for Headers {
    fn from(pairs: [(K, Secret); N]) -> Self {
        pairs.into_iter().collect()
    }
}

impl<'de> Deserialize<'de> for Headers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HeadersVisitor;

        impl<'de> Visitor<'de> for HeadersVisitor {
            type Value = Headers;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping of header names to values, or `k=v` text")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Headers, E> {
                Ok(Headers::from_text(text))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Headers, A::Error> {
                let mut pairs = BTreeMap::new();
                // A value that's null isn't stated, as a `${VAR:-}` that
                // came out empty leaves it, so the header is left out.
                while let Some((name, value)) = entries.next_entry::<String, Option<Secret>>()? {
                    if let Some(value) = value {
                        pairs.insert(name, value);
                    }
                }
                Ok(Headers { pairs, text: None })
            }
        }

        deserializer.deserialize_any(HeadersVisitor)
    }
}

/// Written as a map, each value as the marker, whichever form was read.
impl Serialize for Headers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.pairs.serialize(serializer)
    }
}

impl JsonSchema for Headers {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Headers".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                {
                    "type": "string",
                    "description": "OpenTelemetry's `k=v` text, as `authorization=Bearer%20${TOKEN}`, each value percent-decoded"
                },
                {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            ]
        })
    }
}

/// The resource attributes `telemetry.resource` states: a map, or
/// OpenTelemetry's `k=v` text decoded as the headers' text is. The pairs keep
/// the order they were written in, and a key written twice keeps its first
/// value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resource(Vec<(String, String)>);

impl Resource {
    /// The attributes the `k=v` text `text` names.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        decode_headers(text).into_iter().collect()
    }

    /// The attributes, in their order.
    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// The value of the attribute `key`, when there's one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Whether no attribute is named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Resource {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(pairs: I) -> Self {
        let mut attributes: Vec<(String, String)> = Vec::new();
        for (key, value) in pairs {
            let key = key.into();
            if !attributes.iter().any(|(named, _)| *named == key) {
                attributes.push((key, value.into()));
            }
        }
        Self(attributes)
    }
}

impl<K: Into<String>, V: Into<String>, const N: usize> From<[(K, V); N]> for Resource {
    fn from(pairs: [(K, V); N]) -> Self {
        pairs.into_iter().collect()
    }
}

impl From<BTreeMap<String, String>> for Resource {
    fn from(pairs: BTreeMap<String, String>) -> Self {
        pairs.into_iter().collect()
    }
}

impl<'de> Deserialize<'de> for Resource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ResourceVisitor;

        impl<'de> Visitor<'de> for ResourceVisitor {
            type Value = Resource;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping of attribute keys to values, or `k=v` text")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Resource, E> {
                Ok(Resource::from_text(text))
            }

            /// A text that was one `${VAR}` and came out empty states no
            /// attribute.
            fn visit_unit<E: de::Error>(self) -> Result<Resource, E> {
                Ok(Resource::default())
            }

            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Resource, A::Error> {
                let mut pairs = Vec::new();
                // A value that's null isn't stated, so the attribute is left
                // out.
                while let Some((key, value)) = entries.next_entry::<String, Option<String>>()? {
                    if let Some(value) = value {
                        pairs.push((key, value));
                    }
                }
                Ok(pairs.into_iter().collect())
            }
        }

        deserializer.deserialize_any(ResourceVisitor)
    }
}

/// Written as a map, whichever form was read.
impl Serialize for Resource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl JsonSchema for Resource {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Resource".into()
    }

    /// `null` is read as no attribute, as a `${VAR}` that comes out empty
    /// leaves it, so the schema takes it too.
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                {
                    "type": "string",
                    "description": "OpenTelemetry's `k=v` text, as `service.name=lablet,team=evals`, each value percent-decoded"
                },
                {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                { "type": "null" }
            ]
        })
    }
}

#[cfg(test)]
mod tests;
