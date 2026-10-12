//! Lablet's own secrets and the ones it gives a tool, derived from the
//! config in one place: which variables no process inherits, which names
//! `check` prints, and the values no tool result shows.
//!
//! A credential is defined by the config's fields and never found by a
//! name's pattern. Lablet's own are the model's key, every value of
//! `telemetry.otlp.headers` and of an HTTP server's `headers`, and the user
//! information of `model.base_url`, `telemetry.otlp.endpoint` and a server's
//! `url`; no process a run starts inherits one. The given ones are the
//! entries of `tools.builtin.credentials`, which reach `bash` under their
//! names and nowhere else. Which variable filled each is read off the
//! record the fill kept, so a variable is a credential's only where the
//! config wrote it; in code, a `Secret` in a field is own or given by the
//! field alone. The framework that set any other variable is the party that
//! can name it.
//!
//! The command line's seam still reads the OTLP header and endpoint
//! variables from its environment and sends what they hold, so it hands
//! their values in to be cut, as it hands in what a client key's file holds;
//! every command still inherits those variables.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use lablet_model::Secrets;
use secrecy::{ExposeSecret as _, SecretString};

use lablet_config::{Config, Fill, KeyPath, McpServer, Place, Secret, user_information};
use lablet_otel_env::decode_headers;

/// What a run drops, gives and cuts, derived from its config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    /// Lablet's own credentials, which no process inherits, in the order of
    /// the config's sections.
    pub own: Vec<Credential>,
    /// The credentials given to `bash`, each under the name written under
    /// `tools.builtin.credentials`, in that order.
    pub given: Vec<Credential>,
    /// The variables that filled an own or given field, which every process
    /// a run starts is dropped by name.
    pub dropped: BTreeSet<String>,
    /// The names whose values are cut and nothing else: what the command
    /// line still reads from its environment and sends, which every command
    /// inherits, in the order they're printed.
    pub cut: Vec<Named>,
    /// The values, behind a `Debug` form that shows none of them. Empty
    /// when the run has no executor to hand them to, since nothing would
    /// cut them and holding them would serve nothing.
    pub values: Secrets,
    /// The own and given values alone, which the drop by value matches: a
    /// value the command line hands in to be cut is left out, since every
    /// command still inherits its variable.
    pub dropping: Secrets,
}

/// One credential: the field it fills, where its value came from, whether
/// the value is cut, and who gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The field: `model.api_key`, a header's key, a URL's key, or the
    /// entry under `tools.builtin.credentials`.
    pub field: KeyPath,
    /// Where the field was written, when it was.
    pub place: Option<Place>,
    /// Where the value came from.
    pub source: Source,
    /// Whether the value is cut.
    pub held: Held,
    /// For an own credential, the entry under `tools.builtin.credentials`
    /// that gives it to `bash` too, with its place: the same variable
    /// written there knowingly.
    pub given_by: Option<(KeyPath, Option<Place>)>,
    /// The variables of the environment handed in that hold the value under
    /// another name, which the drop by value took out; filled in by the
    /// root once the environment is computed.
    pub copies: Vec<String>,
    /// What the value is cut as: the value and its bounded parts, so a copy
    /// under another name can be told to be this credential's.
    pub matcher: Secrets,
}

/// A name `check` prints among what's cut, and what became of its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// Where the value comes from.
    pub source: Source,
    /// Whether it's cut.
    pub held: Held,
}

/// Where a value comes from, which is how it's named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A variable, filled into the field or read by the command line's seam.
    Variable(String),
    /// Written in the config, at this key.
    Key(KeyPath),
    /// Set in code, as a `Secret`.
    Code,
}

/// Whether a value is cut, and when it isn't, why not: the reasons a
/// value is silently no secret, which `check` makes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// The value is cut.
    Cut,
    /// The variable isn't set.
    Unset,
    /// The value is empty, or whitespace.
    Empty,
    /// The value is under [`Secrets::MIN_BYTES`], so it isn't cut.
    Short,
}

impl Held {
    /// How a value's not being cut is said after its name, or nothing for
    /// one that's cut.
    fn note(self) -> String {
        match self {
            Self::Cut => String::new(),
            Self::Unset => " (unset, not cut)".to_owned(),
            Self::Empty => " (empty, not cut)".to_owned(),
            Self::Short => format!(" (under {} bytes, not cut)", Secrets::MIN_BYTES),
        }
    }

    /// How a value is held: cut, or why not.
    fn of(value: &str) -> Self {
        if value.trim().is_empty() {
            Self::Empty
        } else if Secrets::new([value.to_owned()]).is_empty() {
            Self::Short
        } else {
            Self::Cut
        }
    }
}

/// Where a setting was written, as a line says it after the name.
fn at(place: Option<Place>) -> String {
    place.map(|place| format!(" ({place})")).unwrap_or_default()
}

impl Credential {
    /// The variable the value came from, when it came from one.
    #[must_use]
    pub fn variable(&self) -> Option<&str> {
        match &self.source {
            Source::Variable(name) => Some(name),
            Source::Key(_) | Source::Code => None,
        }
    }

    /// The name `bash` is given the value under: the entry's name, for a
    /// credential under `tools.builtin.credentials`.
    #[must_use]
    pub fn given_as(&self) -> Option<&str> {
        self.field
            .is_within(&KeyPath::of("tools.builtin.credentials"))
            .then(|| self.field.last_key())
            .flatten()
    }

    /// Whether the value holds `value`, as the cut would take it, so a
    /// variable of the environment holding it is a copy of this credential.
    #[must_use]
    pub fn holds(&self, value: &str) -> bool {
        if self.matcher.is_empty() {
            return false;
        }
        let decoded = percent_decoded(value);
        self.matcher.redacted(value) != value || self.matcher.redacted(&decoded) != decoded
    }

    /// The credential as `check` prints it under `own:`: its variable and
    /// place, the field it fills, and who gets it.
    #[must_use]
    pub fn as_own(&self) -> String {
        let mut line = match &self.source {
            Source::Variable(name) => format!(
                "{name}{}{} -> {}",
                at(self.place),
                self.held.note(),
                self.field
            ),
            Source::Code => format!("{} (set in code){}", self.field, self.held.note()),
            Source::Key(key) => format!(
                "{key}{} holds its value{}",
                at(self.place),
                self.held.note()
            ),
        };
        for copy in &self.copies {
            let _ = write!(line, ", or {copy}, which holds it");
        }
        match &self.given_by {
            Some((key, place)) => {
                let entry = key.last_key().unwrap_or_default();
                let _ = write!(line, "; given to bash by credentials{}", at(*place));
                if !matches!(&self.source, Source::Variable(name) if *name == entry) {
                    let _ = write!(line, " as {entry}");
                }
            }
            None => line.push_str("; no process inherits it"),
        }
        line
    }

    /// The credential as `check` prints it under `given:`: its variable and
    /// place, and the tool that gets it, under the entry's name where that
    /// differs from the variable's.
    #[must_use]
    pub fn as_given(&self) -> String {
        let entry = self.given_as().unwrap_or_default().to_owned();
        let mut line = match &self.source {
            Source::Variable(name) if *name == entry => {
                format!("{name}{}{} -> bash", at(self.place), self.held.note())
            }
            Source::Variable(name) => format!(
                "{name}{}{} -> bash as {entry}",
                at(self.place),
                self.held.note()
            ),
            Source::Code => format!("{} (set in code){} -> bash", self.field, self.held.note()),
            Source::Key(key) => format!(
                "{key}{} holds its value{} -> bash",
                at(self.place),
                self.held.note()
            ),
        };
        for copy in &self.copies {
            let _ = write!(line, ", or {copy}, which holds it");
        }
        line
    }
}

impl Named {
    /// The name alone, which the list is ordered by.
    fn name(&self) -> String {
        match &self.source {
            Source::Variable(name) => name.clone(),
            Source::Key(key) => key.to_string(),
            Source::Code => "set in code".to_owned(),
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

/// A value the command line still reads from its environment and cuts
/// without dropping its variable: the OTLP header and endpoint variables
/// the seam sends, and what a client key's file holds.
#[derive(Debug, Clone, Copy)]
pub struct Handed<'a> {
    /// The variable's name.
    pub name: &'a str,
    /// What it holds, or what the file it names holds.
    pub value: &'a SecretString,
    /// What of the value is cut.
    pub kind: HandedKind,
}

/// What's cut of a value the command line hands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandedKind {
    /// The value whole, as `env` prints it, and each header it decodes to,
    /// which a request carries.
    Headers,
    /// The user information of a URL, in each form a command could print
    /// it; a value with none is no secret.
    Url,
    /// What a file holds: the whole and each line.
    Contents,
}

/// The variables that fill lablet's credentials, each with the field it
/// fills: what a check of the other settings asks before the rest is
/// derived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Variables {
    /// Each variable that fills an own credential, with the field.
    pub own: Vec<(String, KeyPath)>,
    /// Each variable that fills a given credential, with the entry.
    pub given: Vec<(String, KeyPath)>,
}

impl Variables {
    /// The own field the variable `name` fills, when it fills one.
    #[must_use]
    pub fn own_field(&self, name: &str) -> Option<&KeyPath> {
        self.own
            .iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, field)| field)
    }

    /// The entry under `tools.builtin.credentials` the variable `name`
    /// fills, when it fills one.
    #[must_use]
    pub fn given_entry(&self, name: &str) -> Option<&KeyPath> {
        self.given
            .iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, field)| field)
    }
}

/// Which variables fill lablet's credentials in `config`, read off the
/// record the fill kept.
#[must_use]
pub fn variables(config: &Config) -> Variables {
    let derived = derived(config, &[], false);
    let named = |credentials: &[Credential]| -> Vec<(String, KeyPath)> {
        credentials
            .iter()
            .filter_map(|credential| {
                credential
                    .variable()
                    .map(|name| (name.to_owned(), credential.field.clone()))
            })
            .collect()
    };
    Variables {
        own: named(&derived.own),
        given: named(&derived.given),
    }
}

/// Lablet's credentials as `config` defines them, with which variable filled
/// each read off the record the fill kept, and `handed` cut beside them:
/// each named under `cut`, never dropped, and its forms among the values.
///
/// The values are held, in [`Derived::values`], only `with_values`, which is
/// when the run has an executor to hand them to.
#[must_use]
pub fn derived(config: &Config, handed: &[Handed<'_>], with_values: bool) -> Derived {
    let mut set = Set {
        config,
        with_values,
        own: Vec::new(),
        given: Vec::new(),
        cut: Vec::new(),
        values: Vec::new(),
        credentials: 0,
    };
    let model = KeyPath::of("model.api_key");
    match (&config.model.api_key, config.filled().at(&model)) {
        (Some(key), fill) => set.secret(&model, key, fill, Side::Own),
        (None, Some(fill)) => set.unset(&model, fill, Side::Own),
        (None, None) => {}
    }
    if let Some(headers) = &config.telemetry.otlp.headers {
        set.headers(&KeyPath::of("telemetry.otlp.headers"), headers);
    }
    set.url(
        &KeyPath::of("telemetry.otlp.endpoint"),
        config.telemetry.otlp.endpoint.as_deref(),
    );
    set.url(
        &KeyPath::of("model.base_url"),
        config.model.base_url.as_deref(),
    );
    for (index, server) in config.tools.mcp.iter().enumerate() {
        let key = KeyPath::of("tools.mcp").index(index);
        if let McpServer::Http { url, headers, .. } = server {
            set.headers(&key.key("headers"), headers);
            set.url(&key.key("url"), Some(url));
        }
    }
    let credentials = KeyPath::of("tools.builtin.credentials");
    for (name, secret) in &config.tools.builtin.credentials {
        let key = credentials.key(name);
        set.secret(&key, secret, config.filled().at(&key), Side::Given);
    }
    set.credentials = set.values.len();
    for Handed { name, value, kind } in handed {
        set.handed(name, value.expose_secret(), *kind);
    }
    set.finish()
}

/// Which side a credential is on.
#[derive(Clone, Copy)]
enum Side {
    Own,
    Given,
}

/// The set as it's gathered, field by field.
struct Set<'a> {
    config: &'a Config,
    with_values: bool,
    own: Vec<Credential>,
    given: Vec<Credential>,
    cut: Vec<Named>,
    values: Vec<String>,
    /// How many of `values` are own or given, which come first.
    credentials: usize,
}

impl Set<'_> {
    fn push(&mut self, side: Side, credential: Credential) {
        match side {
            Side::Own => self.own.push(credential),
            Side::Given => self.given.push(credential),
        }
    }

    /// A value that's a secret as it stands, held and classified.
    fn value(&mut self, value: &str) -> Held {
        if self.with_values {
            self.values.push(value.to_owned());
        }
        Held::of(value)
    }

    /// A `Secret` at `key`: from each variable `fill` names, or set in
    /// code, or written out.
    fn secret(&mut self, key: &KeyPath, secret: &Secret, fill: Option<&Fill>, side: Side) {
        let value = secret.expose_secret();
        let held = self.value(value);
        let matcher = Secrets::new([value.to_owned()]);
        // A key the text didn't write was set in code, wherever the config
        // came from.
        let sources = match fill {
            Some(fill) if !fill.variables.is_empty() => fill
                .variables
                .iter()
                .map(|variable| Source::Variable(variable.name.clone()))
                .collect(),
            _ if self.config.place_at(key).is_none() => vec![Source::Code],
            _ => vec![Source::Key(key.clone())],
        };
        for source in sources {
            self.push(
                side,
                Credential {
                    field: key.clone(),
                    place: self.config.place_at(key),
                    source,
                    held,
                    given_by: None,
                    copies: Vec::new(),
                    matcher: matcher.clone(),
                },
            );
        }
    }

    /// The field at `key` is unset, since the variable `fill` names is.
    fn unset(&mut self, key: &KeyPath, fill: &Fill, side: Side) {
        for variable in &fill.variables {
            self.push(
                side,
                Credential {
                    field: key.clone(),
                    place: self.config.place_at(key),
                    source: Source::Variable(variable.name.clone()),
                    held: Held::Unset,
                    given_by: None,
                    copies: Vec::new(),
                    matcher: Secrets::default(),
                },
            );
        }
    }

    /// A value the command line hands in: cut in `kind`'s forms, named, and
    /// never dropped.
    fn handed(&mut self, name: &str, value: &str, kind: HandedKind) {
        let forms: Vec<String> = match kind {
            HandedKind::Headers => std::iter::once(value.to_owned())
                .chain(
                    decode_headers(value)
                        .into_iter()
                        .map(|(_, decoded)| decoded),
                )
                .collect(),
            HandedKind::Url => match user_information(value) {
                Some(information) => information_forms(information),
                None => return,
            },
            HandedKind::Contents => vec![value.to_owned()],
        };
        let held = Held::of(&forms[0]);
        if self.with_values {
            self.values.extend(forms);
        }
        self.cut.push(Named {
            source: Source::Variable(name.to_owned()),
            held,
        });
    }

    /// Every value of a `headers` map at `key`: own, from its variables or
    /// written out.
    fn headers(&mut self, key: &KeyPath, headers: &std::collections::BTreeMap<String, Secret>) {
        for (name, value) in headers {
            let at = key.key(name);
            self.secret(&at, value, self.config.filled().at(&at), Side::Own);
        }
    }

    /// The user information of the URL at `key`: the whole `user:password`,
    /// each half, raw and percent-decoded. A variable filled into the
    /// written user information is own, and so is one that gave the URL its
    /// user information from outside the written text, as a variable
    /// holding the whole URL does. A host or a path a variable gave is no
    /// secret.
    fn url(&mut self, key: &KeyPath, url: Option<&str>) {
        let Some(information) = url.and_then(user_information) else {
            return;
        };
        let forms = information_forms(information);
        let held = Held::of(information);
        if self.with_values {
            self.values.extend(forms.iter().cloned());
        }
        let matcher = Secrets::new(forms);
        let fill = self.config.filled().at(key);
        let written_information = fill
            .map(|fill| fill.written.as_str())
            .and_then(user_information);
        let variables: Vec<String> = fill
            .map(|fill| {
                fill.names()
                    .filter(|variable| {
                        written_information.is_none_or(|information| {
                            information.contains(&format!("${{{variable}}}"))
                                || information.contains(&format!("${{{variable}:-"))
                        })
                    })
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let place = self.config.place_at(key);
        if variables.is_empty() {
            self.own.push(Credential {
                field: key.clone(),
                place,
                source: Source::Key(key.clone()),
                held,
                given_by: None,
                copies: Vec::new(),
                matcher: matcher.clone(),
            });
        }
        for variable in variables {
            self.own.push(Credential {
                field: key.clone(),
                place,
                source: Source::Variable(variable),
                held,
                given_by: None,
                copies: Vec::new(),
                matcher: matcher.clone(),
            });
        }
    }

    fn finish(self) -> Derived {
        let Self {
            config: _,
            with_values: _,
            mut own,
            given,
            mut cut,
            values,
            credentials,
        } = self;
        cut.sort_by_key(Named::name);
        cut.dedup();
        let dropping = Secrets::new(values[..credentials].iter().cloned());
        // Under Q5 an own credential may be listed under `credentials` too:
        // the same variable written there knowingly.
        for credential in &mut own {
            if let Source::Variable(name) = &credential.source
                && let Some(entry) = given
                    .iter()
                    .find(|given| given.variable() == Some(name.as_str()))
            {
                credential.given_by = Some((entry.field.clone(), entry.place));
            }
        }
        let dropped = own
            .iter()
            .chain(&given)
            .filter_map(|credential| credential.variable().map(str::to_owned))
            .collect();
        Derived {
            own,
            given,
            dropped,
            cut,
            values: Secrets::new(values),
            dropping,
        }
    }
}

/// The user information of a URL, `user:password` or a user alone, in each
/// form a command could print it: the whole, each half, raw and
/// percent-decoded.
fn information_forms(information: &str) -> Vec<String> {
    let (user, password) = information
        .split_once(':')
        .map_or((information, None), |(user, password)| {
            (user, Some(password))
        });
    let mut forms = Vec::new();
    for part in [Some(information), Some(user), password]
        .into_iter()
        .flatten()
    {
        forms.push(part.to_owned());
        forms.push(percent_decoded(part));
    }
    forms
}

/// `text` with each `%XX` read as the byte it encodes, and a `%` that
/// begins no such pair kept as it is, read as a command's output is read.
#[must_use]
pub fn percent_decoded(text: &str) -> String {
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
