//! Adapter shared kernel: the environment as a carrier of OpenTelemetry
//! context, as the specification's environment carriers page has it.
//!
//! A propagator names its keys as an HTTP header is named, and a variable
//! can't be: the key is carried by the variable its normalised name names
//! ([`variable`]), and a variable named as the key is written is never
//! read. [`EnvExtractor`] reads a context out of an environment, and
//! [`EnvInjector`] writes one into the environment a process starts with.
//! An empty value is none either way, so a child gets no empty
//! `TRACESTATE`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};

use opentelemetry::propagation::{Extractor, Injector};

/// Every variable a propagator lablet serves carries context in: the W3C
/// trace context's and baggage's, and B3's single header and multiple
/// headers. A process lablet starts inherits none of them, so the context
/// it gets is the one injected into it, whichever propagators it's
/// injected through.
pub const CONTEXT_VARIABLES: [&str; 9] = [
    "TRACEPARENT",
    "TRACESTATE",
    "BAGGAGE",
    "B3",
    "X_B3_TRACEID",
    "X_B3_SPANID",
    "X_B3_PARENTSPANID",
    "X_B3_SAMPLED",
    "X_B3_FLAGS",
];

/// The name of the variable that carries `key`, as the specification's
/// environment carriers normalise it: ASCII letters upper-cased, any other
/// character but a digit or `_` made `_`, a leading digit given a `_`
/// before it, and an empty key `_`.
#[must_use]
pub fn variable(key: &str) -> String {
    let mut name: String = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.starts_with(|character: char| character.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// An environment read as a carrier: a key is read from the variable its
/// normalised name names, and a value that's empty or isn't UTF-8 is none.
#[derive(Debug, Clone, Copy)]
pub struct EnvExtractor<'a>(pub &'a BTreeMap<OsString, OsString>);

impl Extractor for EnvExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(OsStr::new(&variable(key)))
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().filter_map(|name| name.to_str()).collect()
    }
}

/// The environment a process starts with, written as a carrier: each key
/// is set as the variable its normalised name names, unless the value is
/// empty or the variable is one of `kept`: the names a config states, which
/// win over what's injected, and the names withheld from the process,
/// which a context would otherwise put back.
#[derive(Debug)]
pub struct EnvInjector<'a> {
    environment: &'a mut BTreeMap<OsString, OsString>,
    kept: &'a BTreeSet<String>,
}

impl<'a> EnvInjector<'a> {
    /// Writes into `environment`, leaving the variables `kept` names as
    /// they are.
    #[must_use]
    pub const fn new(
        environment: &'a mut BTreeMap<OsString, OsString>,
        kept: &'a BTreeSet<String>,
    ) -> Self {
        Self { environment, kept }
    }
}

impl Injector for EnvInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if value.is_empty() {
            return;
        }
        let name = variable(key);
        if !self.kept.contains(&name) {
            self.environment.insert(name.into(), value.into());
        }
    }
}

#[cfg(test)]
mod tests;
