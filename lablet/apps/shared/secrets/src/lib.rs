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
//! What the file `telemetry.otlp.tls.client_key` names holds is cut too,
//! where the command line reads it: a file and no variable, so nothing is
//! dropped for it, and it's handed in by the root that read it.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use base64::Engine as _;

use lablet_model::Secrets;

use lablet_config::{
    Config, Fill, Headers, KeyPath, McpServer, Place, Secret, came_out_unstated, user_information,
};

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
    /// The values, behind a `Debug` form that shows none of them, held for
    /// every run: the loop cuts them out of a provider error's message, and
    /// every executor out of its results, so a run with no tools needs them
    /// as much as one with.
    pub values: Secrets,
    /// The values the drop by value matches, which are the same values: a
    /// variable of the environment handed in that holds one under another
    /// name is dropped.
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

/// Where a value comes from, which is how it's named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A variable, filled into the field.
    Variable(String),
    /// Written in the config, at this key.
    Key(KeyPath),
    /// Set in code, as a `Secret`.
    Code,
    /// What the file the field names holds, which the root read.
    Contents,
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
            Source::Key(_) | Source::Code | Source::Contents => None,
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
            // A file's contents reach no process unless a `credentials` entry
            // gives them, so the line says what's cut, and who gets it then.
            Source::Contents => {
                let mut line = format!(
                    "{}{} -> its file's contents{}",
                    self.field,
                    at(self.place),
                    self.held.note()
                );
                for copy in &self.copies {
                    let _ = write!(line, ", or {copy}, which holds it");
                }
                if self.held == Held::Cut {
                    line.push_str("; cut");
                }
                if let Some((key, place)) = &self.given_by {
                    let entry = key.last_key().unwrap_or_default();
                    let _ = write!(
                        line,
                        "; given to bash by credentials{} as {entry}",
                        at(*place)
                    );
                }
                return line;
            }
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
            // A `${VAR:-}` that came out empty gives nothing, which the line
            // says in place of a name `bash` doesn't get.
            Source::Variable(name) if self.held == Held::Unset => {
                return format!("none: {name}{} is unset", at(self.place));
            }
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
            Source::Contents => format!(
                "{}{} -> its file's contents{} -> bash",
                self.field,
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
    let derived = derived(config, None);
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
/// each read off the record the fill kept, and `client_key`, what the file
/// `telemetry.otlp.tls.client_key` names holds where the root read it, cut
/// beside them as an own credential that no variable gave.
#[must_use]
pub fn derived(config: &Config, client_key: Option<&str>) -> Derived {
    let mut set = Set {
        config,
        own: Vec::new(),
        given: Vec::new(),
        given_values: Vec::new(),
        values: Vec::new(),
    };
    let model = KeyPath::of("model.api_key");
    match (&config.model.api_key, config.filled().at(&model)) {
        (Some(key), fill) => set.secret(&model, key, fill, Side::Own),
        (None, Some(fill)) => set.unset(&model, fill, Side::Own),
        (None, None) => {}
    }
    let headers = KeyPath::of("telemetry.otlp.headers");
    match (&config.telemetry.otlp.headers, config.filled().at(&headers)) {
        (Some(stated), _) => set.otlp_headers(&headers, stated),
        (None, Some(fill)) => set.unset(&headers, fill, Side::Own),
        (None, None) => {}
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
    // An entry whose `${VAR:-}` came out empty isn't in the map, and
    // `given:` says so.
    set.unset_within(&credentials, &config.tools.builtin.credentials, Side::Given);
    if let Some(contents) = client_key {
        set.contents(&KeyPath::of("telemetry.otlp.tls.client_key"), contents);
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
    own: Vec<Credential>,
    given: Vec<Credential>,
    /// The value of each given credential, in the order of `given`: empty
    /// for one that came out unset.
    given_values: Vec<String>,
    values: Vec<String>,
}

impl Set<'_> {
    fn push(&mut self, side: Side, credential: Credential) {
        self.push_valued(side, credential, "");
    }

    /// `credential`, with the value it gives, which a given one keeps so an
    /// own credential it copies can be told.
    fn push_valued(&mut self, side: Side, credential: Credential, value: &str) {
        match side {
            Side::Own => self.own.push(credential),
            Side::Given => {
                self.given.push(credential);
                self.given_values.push(value.to_owned());
            }
        }
    }

    /// A value that's a secret as it stands, held and classified.
    fn value(&mut self, value: &str) -> Held {
        self.values.push(value.to_owned());
        Held::of(value)
    }

    /// A `Secret` at `key`: from each variable `fill` names, or set in
    /// code, or written out.
    fn secret(&mut self, key: &KeyPath, secret: &Secret, fill: Option<&Fill>, side: Side) {
        let value = secret.expose_secret();
        let held = self.value(value);
        let matcher = Secrets::new([value.to_owned()]);
        // A key the text didn't write was set in code, wherever the config
        // came from, and so is one whose `${VAR}` came out unstated, its
        // variable unset or set to nothing.
        let sources = match fill {
            Some(fill) if came_out_unstated(fill) => vec![Source::Code],
            Some(fill) if !fill.variables.is_empty() => fill
                .variables
                .iter()
                .map(|variable| Source::Variable(variable.name.clone()))
                .collect(),
            _ if self.config.place_at(key).is_none() => vec![Source::Code],
            _ => vec![Source::Key(key.clone())],
        };
        for source in sources {
            self.push_valued(
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
                value,
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

    /// What the file the setting at `key` names holds: own, cut whole and by
    /// each line, as a value with several lines is, from no variable.
    ///
    /// A PEM's armour, its `-----BEGIN` and `-----END` lines, is every
    /// PEM's, so it's left out: cut, it would take any other key's or
    /// certificate's for this one's.
    fn contents(&mut self, key: &KeyPath, contents: &str) {
        let body = contents
            .lines()
            .filter(|line| !line.starts_with("-----BEGIN ") && !line.starts_with("-----END "))
            .collect::<Vec<_>>()
            .join("\n");
        let held = self.value(&body);
        self.own.push(Credential {
            field: key.clone(),
            place: self.config.place_at(key),
            source: Source::Contents,
            held,
            given_by: None,
            copies: Vec::new(),
            matcher: Secrets::new([body]),
        });
    }

    /// Every value of a `headers` map at `key`: own, from its variables or
    /// written out. An entry whose `${VAR:-}` came out empty is unset, and
    /// named so.
    fn headers(&mut self, key: &KeyPath, headers: &std::collections::BTreeMap<String, Secret>) {
        for (name, value) in headers {
            let at = key.key(name);
            self.secret(&at, value, self.config.filled().at(&at), Side::Own);
        }
        self.unset_within(key, headers, Side::Own);
    }

    /// `telemetry.otlp.headers` in either form. The text form is one
    /// setting: each variable that filled it is own at the field, and what's
    /// cut is the text whole and each value it decodes to, since a request
    /// carries the values and a command could print the text.
    fn otlp_headers(&mut self, key: &KeyPath, headers: &Headers) {
        let Some(text) = headers.text() else {
            let pairs: std::collections::BTreeMap<String, Secret> = headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            self.headers(key, &pairs);
            return;
        };
        let text = text.expose_secret();
        let held = Held::of(text);
        let mut forms = vec![text.to_owned()];
        forms.extend(
            headers
                .iter()
                .map(|(_, value)| value.expose_secret().to_owned()),
        );
        self.values.extend(forms.iter().cloned());
        let matcher = Secrets::new(forms);
        let fill = self.config.filled().at(key);
        let sources: Vec<Source> = match fill {
            Some(fill) if !fill.variables.is_empty() => fill
                .variables
                .iter()
                .map(|variable| Source::Variable(variable.name.clone()))
                .collect(),
            _ if self.config.place_at(key).is_none() => vec![Source::Code],
            _ => vec![Source::Key(key.clone())],
        };
        for source in sources {
            self.own.push(Credential {
                field: key.clone(),
                place: self.config.place_at(key),
                source,
                held,
                given_by: None,
                copies: Vec::new(),
                matcher: matcher.clone(),
            });
        }
    }

    /// Each entry of the map at `key` the record says a `${VAR}` filled and
    /// the map doesn't hold: its variable came out unset or empty, so the
    /// entry isn't stated, which `check` says.
    fn unset_within<V>(
        &mut self,
        key: &KeyPath,
        held: &std::collections::BTreeMap<String, V>,
        side: Side,
    ) {
        let unset: Vec<(KeyPath, Fill)> = self
            .config
            .filled()
            .within(key)
            .filter(|(at, fill)| {
                *at != key
                    && !fill.variables.is_empty()
                    && at.last_key().is_some_and(|name| !held.contains_key(name))
            })
            .map(|(at, fill)| (at.clone(), fill.clone()))
            .collect();
        for (at, fill) in unset {
            self.unset(&at, &fill, side);
        }
    }

    /// The user information of the URL at `key`: the whole `user:password`,
    /// each half, raw and percent-decoded, and the `Basic` credentials a
    /// client makes of it. A variable filled into the
    /// written user information is own, and so is one that gave the URL its
    /// user information from outside the written text, as a variable
    /// holding the whole URL does. A host or a path a variable gave is no
    /// secret. A URL the text didn't write was set in code.
    fn url(&mut self, key: &KeyPath, url: Option<&str>) {
        let Some(information) = url.and_then(user_information) else {
            return;
        };
        let forms = information_forms(information);
        let held = Held::of(information);
        self.values.extend(forms.iter().cloned());
        let matcher = Secrets::new(forms);
        let place = self.config.place_at(key);
        // A URL with no place was set in code, over whatever a fill left.
        let fill = self.config.filled().at(key).filter(|_| place.is_some());
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
        if variables.is_empty() {
            self.own.push(Credential {
                field: key.clone(),
                place,
                source: if place.is_some() {
                    Source::Key(key.clone())
                } else {
                    Source::Code
                },
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
            mut own,
            given,
            given_values,
            values,
        } = self;
        let dropping = Secrets::new(values.iter().cloned());
        // Under Q5 an own credential may be listed under `credentials` too:
        // the same variable written there knowingly, or another that holds
        // its value, which gives `bash` the value as surely.
        for credential in &mut own {
            let by_name = match &credential.source {
                Source::Variable(name) => given
                    .iter()
                    .find(|given| given.variable() == Some(name.as_str())),
                Source::Key(_) | Source::Code | Source::Contents => None,
            };
            let entry = by_name.or_else(|| {
                given
                    .iter()
                    .zip(&given_values)
                    .find(|(_, value)| !value.is_empty() && credential.holds(value))
                    .map(|(given, _)| given)
            });
            if let Some(entry) = entry {
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
            values: Secrets::new(values),
            dropping,
        }
    }
}

/// The user information of a URL, `user:password` or a user alone, in each
/// form a command could print it or a server could repeat it: the whole,
/// each half, raw and percent-decoded, and the `Authorization: Basic`
/// credentials an HTTP client makes of it, the Anthropic adapter's and
/// reqwest's alike, with and without the scheme's word. The `Basic` forms
/// join only when the whole user information is long enough to be cut, since
/// `check`'s note speaks of the whole; a half or a decoded form under 16
/// bytes is dropped by `Secrets::new`, as every registered form is.
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
    if Held::of(information) != Held::Cut {
        return forms;
    }
    // The halves decoded and joined by one `:`, a user alone with an empty
    // password, as the client encodes them before it sends.
    let basic = base64::engine::general_purpose::STANDARD.encode(format!(
        "{}:{}",
        percent_decoded(user),
        percent_decoded(password.unwrap_or_default())
    ));
    forms.push(format!("Basic {basic}"));
    forms.push(basic);
    forms
}

/// `text` with each `%XX` read as the byte it encodes, and a `%` that
/// begins no such pair, two hex digits, kept as it is, read as a command's
/// output is read and as the adapter and reqwest read a URL.
#[must_use]
pub fn percent_decoded(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some(at) = rest.iter().position(|&byte| byte == b'%') {
        bytes.extend_from_slice(&rest[..at]);
        let encoded = rest
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            // `from_str_radix` would read a sign, which no URL reader does.
            .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
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
