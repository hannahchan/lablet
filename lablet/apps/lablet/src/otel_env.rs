//! The seam where lablet reads the OpenTelemetry environment, once, from the
//! environment a check or a build reads everything else from, parsing each
//! variable as the specification parses it. The modules that build from the
//! values take them from here and state each one on the SDK, so the crates'
//! own reading of the process environment never decides.
//!
//! The specification's parsing, for every variable read here: an empty value
//! is unset; a name chosen from a list is matched in any case, trimmed, and
//! a list has its duplicates dropped; a Boolean is `true` only when it's
//! `true` in any case; and a value that can't be used is warned about,
//! naming the variable and, unless it may hold a secret, the value, and is
//! then ignored, so the next source decides.

mod context;
mod exporter;
mod sdk;

use std::num::NonZeroUsize;
use std::time::Duration;

pub(crate) use context::Context;
pub(crate) use exporter::Exporter;
pub(crate) use sdk::Sdk;

use crate::config::Env;

/// The variables the seam reads, from one environment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OtelEnv {
    /// The SDK's own: the sampler, the span limits and the batch processors.
    pub(crate) sdk: Sdk,
    /// The OTLP exporter's.
    pub(crate) exporter: Exporter,
    /// The resource's and the inbound context's.
    pub(crate) context: Context,
}

impl OtelEnv {
    /// Reads the seam's variables from `env`, warning of each value that
    /// can't be used.
    pub(crate) fn read(env: Env<'_>) -> Self {
        let variables = Variables(env);
        Self {
            sdk: Sdk::read(&variables),
            exporter: Exporter::read(&variables),
            context: Context::read(&variables),
        }
    }
}

/// The environment, as the seam reads a variable of it.
pub(crate) struct Variables<'a>(Env<'a>);

impl Variables<'_> {
    /// What `name` holds read as a `T`, or nothing when it's unset, empty,
    /// or holds nothing a `T` can use. Each part of the value that's
    /// ignored is warned about.
    pub(crate) fn get<T: Parse>(&self, name: &str) -> Option<T> {
        let value = (self.0)(name)?;
        let Some(text) = value.to_str() else {
            tracing::warn!("`{name}` holds what isn't UTF-8, so it's ignored");
            return None;
        };
        if text.is_empty() {
            return None;
        }
        T::parse(text, &mut |part, why| {
            if T::SECRET {
                tracing::warn!("`{name}` holds a value that {why}");
            } else {
                tracing::warn!("`{name}` holds `{part}`, which {why}");
            }
        })
    }
}

/// What a variable's value is read as.
pub(crate) trait Parse: Sized {
    /// Whether the value may hold a secret, so a warning names the variable
    /// alone.
    const SECRET: bool = false;

    /// What `text`, which isn't empty, reads as. Each part of it that's
    /// ignored is handed to `ignored` with why, as a clause that ends the
    /// warning: "isn't a whole number, so it's ignored".
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self>;
}

/// The text as it's written.
impl Parse for String {
    fn parse(text: &str, _ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        Some(text.to_owned())
    }
}

/// The specification's Boolean: `true` in any case is true, and anything
/// but `false` in any case is warned about and read as false. A caller
/// reads a variable that gives nothing as false.
impl Parse for bool {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        if text.eq_ignore_ascii_case("true") {
            Some(true)
        } else if text.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            ignored(text, "isn't `true` or `false`, so it's read as `false`");
            None
        }
    }
}

/// A count that may be zero, as a limit is.
impl Parse for u32 {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        whole(text, ignored, "isn't a whole number, so it's ignored")
    }
}

/// A size that must be more than zero, as a queue's or a batch's is.
impl Parse for NonZeroUsize {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        whole(
            text,
            ignored,
            "isn't a whole number above zero, so it's ignored",
        )
    }
}

/// A duration, in whole milliseconds.
impl Parse for Duration {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        whole(
            text,
            ignored,
            "isn't a whole number of milliseconds, so it's ignored",
        )
        .map(Duration::from_millis)
    }
}

/// `text` as a whole number of digits alone, or nothing, said with `why`.
/// Rust's own parsing takes a leading `+`, which the specification's
/// integers don't have.
fn whole<T: std::str::FromStr>(
    text: &str,
    ignored: &mut dyn FnMut(&str, &str),
    why: &str,
) -> Option<T> {
    let parsed = text
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| text.parse().ok())
        .flatten();
    if parsed.is_none() {
        ignored(text, why);
    }
    parsed
}

/// A value of a variable that names one of a fixed set of choices, as an
/// enum of the specification's does.
pub(crate) trait Choice: Sized {
    /// The choice `name` spells, lower-cased and trimmed, when it's one
    /// lablet serves.
    fn named(name: &str) -> Option<Self>;
}

/// The one choice a variable names, matched in any case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Named<T>(pub(crate) T);

impl<T: Choice> Parse for Named<T> {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        let named = T::named(&text.trim().to_ascii_lowercase()).map(Named);
        if named.is_none() {
            ignored(text, "isn't one lablet serves, so it's ignored");
        }
        named
    }
}

/// A comma list of choices, each trimmed and matched in any case, with
/// duplicates dropped. `none` alone is the empty list, and beside another
/// name it's ignored. A list that names nothing lablet serves gives
/// nothing, as an unset variable does, since what isn't recognised is
/// ignored rather than read as `none`.
impl<T: Choice + PartialEq> Parse for Vec<T> {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        let mut names: Vec<String> = Vec::new();
        for name in text.split(',').map(|name| name.trim().to_ascii_lowercase()) {
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        if names == ["none"] {
            return Some(Vec::new());
        }
        let mut chosen = Vec::new();
        for name in names {
            if name == "none" {
                ignored(&name, "is beside another value, so it's ignored");
                continue;
            }
            match T::named(&name) {
                Some(choice) if !chosen.contains(&choice) => chosen.push(choice),
                Some(_) => {}
                None => ignored(&name, "isn't one lablet serves, so it's ignored"),
            }
        }
        (!chosen.is_empty()).then_some(chosen)
    }
}

#[cfg(test)]
mod tests;
