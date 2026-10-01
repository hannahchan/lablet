//! Lablet's own secrets, derived from the config in one place: the
//! variables no command inherits, the names whose values no tool result
//! shows, and the values themselves.
//!
//! A secret is defined by the config's fields and never found by a name's
//! pattern: the model's key; every value of `telemetry.otlp.headers` and of
//! an HTTP server's `headers`, written or substituted; the user information
//! of `model.base_url`, `telemetry.otlp.endpoint` and a server's `url`; and
//! every variable substituted into `tools.builtin.env` or a stdio server's
//! `env`. The framework that set any other variable is the party that can
//! name it, and a pattern would cut path-valued and URL-valued variables
//! from every record. The one secret found rather than told is the OTLP
//! header variables the exporter reads from the environment: inherited by
//! every command, since the exporter in a child reads them too, and cut.

use std::collections::BTreeSet;
use std::fmt;

use lablet_model::Secrets;
use lablet_telemetry_otel::decode_headers;

use crate::config::{Config, Env, KeyPath, McpServer, Substituted};
use crate::otlp;

/// What a run withholds and cuts, derived from its config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Derived {
    /// The variables no command inherits.
    pub(crate) withheld: BTreeSet<String>,
    /// The names whose values no tool result shows, in the order they're
    /// printed. A variable here and not in `withheld` is inherited by every
    /// command and cut, which is the class of the OTLP header variables the
    /// exporter reads from the environment.
    pub(crate) cut: Vec<Named>,
    /// The values, behind a `Debug` form that shows none of them. Empty
    /// when the run has no executor to hand them to, since nothing would
    /// cut them and holding them would serve nothing.
    pub(crate) values: Secrets,
}

/// A name `check` prints among what's cut, and what became of its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    pub(crate) source: Source,
    pub(crate) held: Held,
}

/// Where a cut value comes from, which is how it's named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A variable of lablet's environment.
    Variable(String),
    /// A value written in the config, at this key.
    Key(KeyPath),
}

/// Whether a value is cut, and when it isn't, why not: the reasons a
/// value is silently no secret, which `check` makes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Held {
    /// The value is cut.
    Cut,
    /// The variable isn't set.
    Unset,
    /// The value is empty, or whitespace.
    Empty,
    /// The value is under [`Secrets::MIN_BYTES`], so it isn't cut.
    Short,
}

impl Named {
    /// The name alone, which the list is ordered by.
    fn name(&self) -> String {
        match &self.source {
            Source::Variable(name) => name.clone(),
            Source::Key(key) => key.to_string(),
        }
    }
}

impl fmt::Display for Named {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())?;
        match self.held {
            Held::Cut => Ok(()),
            Held::Unset => f.write_str(" (not set, not cut)"),
            Held::Empty => f.write_str(" (empty, not cut)"),
            Held::Short => write!(f, " (under {} bytes, not cut)", Secrets::MIN_BYTES),
        }
    }
}

/// Lablet's secrets as `written` and `real`, the same config with `${VAR}`
/// substituted, define them, with each variable's value read from `env`.
///
/// Every variable named is read, to say whether its value is cut, as the
/// check of the key's variable reads it. The values are held, in
/// [`Derived::values`], only `with_values`, which is when the run has an
/// executor to hand them to.
pub(crate) fn derived(
    written: &Config,
    real: &Substituted,
    env: Env<'_>,
    with_values: bool,
) -> Derived {
    let mut set = Set {
        env,
        with_values,
        withheld: BTreeSet::new(),
        cut: Vec::new(),
        values: Vec::new(),
    };
    if let Some(variable) = real.model.key_variable() {
        set.variable(variable);
    }
    set.headers(
        &KeyPath::of("telemetry.otlp.headers"),
        &real.telemetry.otlp.headers,
        real,
    );
    set.url(
        &KeyPath::of("telemetry.otlp.endpoint"),
        written.telemetry.otlp.endpoint.as_deref(),
        real.telemetry.otlp.endpoint.as_deref(),
        real,
    );
    set.url(
        &KeyPath::of("model.base_url"),
        written.model.base_url.as_deref(),
        real.model.base_url.as_deref(),
        real,
    );
    set.env(&KeyPath::of("tools.builtin.env"), real);
    for (index, server) in real.tools.mcp.iter().enumerate() {
        let key = KeyPath::of("tools.mcp").index(index);
        match server {
            McpServer::Http { url, headers, .. } => {
                set.headers(&key.key("headers"), headers, real);
                let written_url = match written.tools.mcp.get(index) {
                    Some(McpServer::Http { url, .. }) => Some(url.as_str()),
                    Some(McpServer::Stdio { .. }) | None => None,
                };
                set.url(&key.key("url"), written_url, Some(url), real);
            }
            McpServer::Stdio { .. } => set.env(&key.key("env"), real),
        }
    }
    for variable in otlp::HEADER_VARIABLES {
        set.inherited(variable);
    }
    set.finish()
}

/// The set as it's gathered, field by field.
struct Set<'a> {
    env: Env<'a>,
    with_values: bool,
    withheld: BTreeSet<String>,
    cut: Vec<Named>,
    values: Vec<String>,
}

impl Set<'_> {
    /// A variable lablet reads a secret from: withheld, named, and its
    /// value read.
    fn variable(&mut self, name: &str) {
        if !self.withheld.insert(name.to_owned()) {
            return;
        }
        let held = match (self.env)(name) {
            // The environment is lossily read as a command's output is, so
            // a value that isn't UTF-8 is cut from the output of the same
            // bytes.
            Some(value) => self.value(&value.to_string_lossy()),
            None => Held::Unset,
        };
        self.cut.push(Named {
            source: Source::Variable(name.to_owned()),
            held,
        });
    }

    /// A variable every command inherits whose value is a secret all the
    /// same, when it's set: the value whole, which `env` prints, and each
    /// header it decodes to, which a request carries, are cut, and the name
    /// is listed without being withheld.
    fn inherited(&mut self, name: &str) {
        let Some(value) = (self.env)(name) else {
            return;
        };
        let value = value.to_string_lossy();
        let held = self.value(&value);
        for (_, decoded) in decode_headers(&value) {
            self.value(&decoded);
        }
        self.cut.push(Named {
            source: Source::Variable(name.to_owned()),
            held,
        });
    }

    /// A value that's a secret as it stands, held and classified.
    fn value(&mut self, value: &str) -> Held {
        if self.with_values {
            self.values.push(value.to_owned());
        }
        if value.trim().is_empty() {
            Held::Empty
        } else if Secrets::new([value.to_owned()]).is_empty() {
            Held::Short
        } else {
            Held::Cut
        }
    }

    /// Every value of a `headers` map at `key`: cut as substituted, and
    /// named by the variables substituted into it, or by its key when it
    /// was written out.
    fn headers(
        &mut self,
        key: &KeyPath,
        headers: &std::collections::BTreeMap<String, String>,
        real: &Substituted,
    ) {
        for (name, value) in headers {
            let at = key.key(name);
            let held = self.value(value);
            let variables: Vec<String> = real.replaced_within(&at).map(str::to_owned).collect();
            if variables.is_empty() {
                self.cut.push(Named {
                    source: Source::Key(at),
                    held,
                });
            }
            for variable in variables {
                self.variable(&variable);
            }
        }
    }

    /// The user information of the URL at `key`, as substituted: the whole
    /// `user:password`, each half, raw and percent-decoded. A variable
    /// substituted into the written user information is withheld, and so
    /// is one that gave the URL its user information from outside the
    /// written text, as a variable holding the whole URL does. A host or a
    /// path a variable gave is no secret.
    fn url(
        &mut self,
        key: &KeyPath,
        written: Option<&str>,
        real: Option<&str>,
        substituted: &Substituted,
    ) {
        let Some(information) = real.and_then(user_information) else {
            return;
        };
        let (user, password) = information
            .split_once(':')
            .map_or((information, None), |(user, password)| {
                (user, Some(password))
            });
        let mut held = Held::Cut;
        for part in [Some(information), Some(user), password]
            .into_iter()
            .flatten()
        {
            let whole = part == information;
            for form in [part.to_owned(), percent_decoded(part)] {
                let classified = self.value(&form);
                if whole && form == information {
                    held = classified;
                }
            }
        }
        let written_information = written.and_then(user_information);
        let variables: Vec<String> = substituted
            .replaced_within(key)
            .filter(|variable| {
                written_information
                    .is_none_or(|information| information.contains(&format!("${{{variable}}}")))
            })
            .map(str::to_owned)
            .collect();
        if variables.is_empty() {
            self.cut.push(Named {
                source: Source::Key(key.clone()),
                held,
            });
        }
        for variable in variables {
            self.variable(&variable);
        }
    }

    /// Every variable substituted into the `env` map at `key`. A value
    /// written out is what a command starts with and no secret.
    fn env(&mut self, key: &KeyPath, real: &Substituted) {
        let variables: Vec<String> = real.replaced_within(key).map(str::to_owned).collect();
        for variable in variables {
            self.variable(&variable);
        }
    }

    fn finish(self) -> Derived {
        let Self {
            withheld,
            mut cut,
            values,
            with_values: _,
            env: _,
        } = self;
        cut.sort_by_key(Named::name);
        Derived {
            withheld,
            cut,
            values: Secrets::new(values),
        }
    }
}

/// The user information of `url`, the `user:password` before the `@` of
/// its authority, when it has one. A short reading of RFC 3986 rather than
/// a URL crate: the authority runs from after `//` to the first `/`, `?` or
/// `#`, and the user information is what's before the last `@` in it.
pub(crate) fn user_information(url: &str) -> Option<&str> {
    let (_, after_scheme) = url.split_once("://")?;
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let at = authority.rfind('@')?;
    Some(&authority[..at])
}

/// `url` with its user information and the `@` after it left out.
pub(crate) fn without_user_information(url: &str) -> String {
    match user_information(url) {
        Some(information) => url.replacen(&format!("{information}@"), "", 1),
        None => url.to_owned(),
    }
}

/// `text` with each `%XX` read as the byte it encodes, and a `%` that
/// begins no such pair kept as it is, read as a command's output is read.
fn percent_decoded(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some(at) = rest.iter().position(|&byte| byte == b'%') {
        bytes.extend_from_slice(&rest[..at]);
        let encoded = rest
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = encoded {
            bytes.push(byte);
            rest = &rest[at + 3..];
        } else {
            bytes.push(b'%');
            rest = &rest[at + 1..];
        }
    }
    bytes.extend_from_slice(rest);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests;
