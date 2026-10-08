//! The specification's parsing of the OpenTelemetry environment's
//! variables, which the seam that reads them is built on. Each root reads
//! the environment through it, so a variable parses alike whichever root
//! reads it.
//!
//! The specification's parsing, for every variable read here: an empty value
//! is unset; a name chosen from a list is matched in any case, trimmed, and
//! a list has its duplicates dropped; a Boolean is `true` only when it's
//! `true` in any case; and a value that can't be used is warned about,
//! naming the variable and, unless it may hold a secret, the value, and is
//! then ignored, so the next source decides.

use std::num::NonZeroUsize;
use std::time::Duration;

use lablet_config::Env;

/// The target of this crate's warnings, the module they were written in
/// before the kernels were split out, so a line of the diagnostic log, and
/// a `RUST_LOG` directive that names it, are as they were.
const TARGET: &str = "lablet::otel_env";

/// The GenAI instrumentation's capture variable, which both roots read:
/// whether content reaches the telemetry when the config says nothing.
pub const CAPTURE_CONTENT: &str = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT";

/// The OTLP exporter's header variables, generic and for each signal,
/// which are secrets whenever they're set, in both modes.
pub const HEADER_VARIABLES: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
];

/// The OTLP exporter's endpoint variables, generic and for each signal,
/// whose user information is a secret in both modes.
pub const ENDPOINT_VARIABLES: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
];

/// What [`CAPTURE_CONTENT`] in `env` says of content capture, or `None`
/// when it says nothing. A value that isn't a Boolean is warned about.
pub fn capture_content(env: Env<'_>) -> Option<bool> {
    Variables(env).get(CAPTURE_CONTENT)
}

/// The environment, as the seam reads a variable of it.
pub struct Variables<'a>(pub Env<'a>);

impl Variables<'_> {
    /// What `name` holds read as a `T`, or nothing when it's unset, empty,
    /// or holds nothing a `T` can use. Each part of the value that's
    /// ignored is warned about.
    pub fn get<T: Parse>(&self, name: &str) -> Option<T> {
        let value = (self.0)(name)?;
        let Some(text) = value.to_str() else {
            tracing::warn!(target: TARGET, "`{name}` holds what isn't UTF-8, so it's ignored");
            return None;
        };
        if text.is_empty() {
            return None;
        }
        T::parse(text, &mut |part, why| {
            if T::SECRET {
                tracing::warn!(target: TARGET, "`{name}` holds a value that {why}");
            } else {
                tracing::warn!(target: TARGET, "`{name}` holds `{part}`, which {why}");
            }
        })
    }
}

/// What a variable's value is read as.
pub trait Parse: Sized {
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

/// The specification's Timeout: whole milliseconds, as a duration is, but
/// 0 is no limit rather than none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeout(pub Duration);

impl Timeout {
    /// What a timeout of 0 is given as. The clients add a timeout to the
    /// clock, so no limit is stated as the very long time the specification
    /// allows in its place, its own example: the most milliseconds a 32-bit
    /// integer holds, 2^31 - 1, about 24.8 days.
    pub const NO_LIMIT: Duration = Duration::from_millis(2_147_483_647);
}

impl Parse for Timeout {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        Duration::parse(text, ignored).map(|timeout| {
            Timeout(if timeout.is_zero() {
                Self::NO_LIMIT
            } else {
                timeout
            })
        })
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
pub trait Choice: Sized {
    /// The choice `name` spells, lower-cased and trimmed, when it's one
    /// lablet serves.
    fn named(name: &str) -> Option<Self>;
}

/// The one choice a variable names, matched in any case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named<T>(pub T);

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

/// The headers `value` names, as the exporter reads them from
/// `OTEL_EXPORTER_OTLP_HEADERS` and its per-signal forms: `name=value`
/// pairs between commas, each trimmed, with the value percent-decoded. A
/// pair whose name or value is empty is left out, and a value whose
/// percent-escapes don't decode is kept as it's written.
#[must_use]
pub fn decode_headers(value: &str) -> Vec<(String, String)> {
    value
        .split_terminator(',')
        .map(str::trim)
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let decoded = percent_decoded(value.trim()).unwrap_or_else(|| value.to_owned());
            (!name.trim().is_empty() && !decoded.is_empty())
                .then(|| (name.trim().to_owned(), decoded))
        })
        .collect()
}

/// `value` with each `%xx` replaced by its byte, or nothing when an escape
/// is cut short, isn't hex, or the bytes aren't UTF-8.
fn percent_decoded(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut bytes = Vec::new();
    let mut chars = value.chars();
    loop {
        let next = chars.next();
        if next == Some('%') {
            let escape = [chars.next()?, chars.next()?];
            bytes.push(u8::from_str_radix(&escape.iter().collect::<String>(), 16).ok()?);
            continue;
        }
        if !bytes.is_empty() {
            decoded.push_str(std::str::from_utf8(&bytes).ok()?);
            bytes.clear();
        }
        match next {
            Some(char) => decoded.push(char),
            None => return Some(decoded),
        }
    }
}

#[cfg(test)]
mod tests;
