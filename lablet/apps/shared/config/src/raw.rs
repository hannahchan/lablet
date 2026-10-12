//! A config's text read into a tree, with the line each setting is on:
//! what an override edits, and what `${VAR}` is filled into, before
//! anything is read into the config's types.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use serde_saphyr::Spanned;

use super::de::{At, Fault, FaultKind, listed};
use super::key::{KeyPath, Place, Places, place_in};
use super::shown::is_credential;
use super::substitute::{self, Filled};
use super::{Config, ConfigError, Format, shown};

/// A config's text, read into a tree of values, before the tree is read as
/// a config.
///
/// It's what an override edits and the fill works on: [`RawConfig::set`]
/// states one setting over what the text states, [`RawConfig::substituted`]
/// fills each `${VAR}` from where the caller says, and [`RawConfig::config`]
/// reads the tree into a [`Config`], whose refusals name the key and the
/// line of what they refuse, and show a filled setting as the text wrote
/// it. A text that holds a `${` wants the fill: a `Config` read without one
/// is refused.
#[derive(Debug, Clone, PartialEq)]
pub struct RawConfig {
    /// A mapping of sections, always.
    tree: Value,
    places: BTreeMap<KeyPath, Place>,
    source: Option<PathBuf>,
    /// What the fill changed, and whether it ran.
    filled: Filled,
}

impl RawConfig {
    /// The tree `text` holds.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Syntax`] when the text isn't in `format`, when
    /// it writes a key twice, and when it isn't a mapping of sections.
    pub fn from_str(text: &str, format: Format) -> Result<Self, ConfigError> {
        let syntax = |reason: String| ConfigError::Syntax { format, reason };
        let (tree, places) = match format {
            Format::Yaml => yaml(text).map_err(syntax)?,
            Format::Json => json(text).map_err(syntax)?,
        };
        let tree = match (tree, format) {
            (tree @ Value::Object(_), _) => tree,
            // YAML reads a text of nothing, or of comments alone, as null.
            (Value::Null, Format::Yaml) => Value::Object(Map::new()),
            _ => {
                return Err(syntax(
                    "a config is a mapping of sections, such as `run:` and `model:`".to_owned(),
                ));
            }
        };
        Ok(Self {
            tree,
            places,
            source: None,
            filled: Filled::default(),
        })
    }

    /// The tree the file at `path` holds, in the format its name says.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::UnknownFormat`] when the file's name ends in
    /// none of `.yaml`, `.yml` and `.json`, [`ConfigError::Unreadable`]
    /// when the file can't be read, and what [`RawConfig::from_str`]
    /// returns for what it holds.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let shown = || path.display().to_string();
        let format =
            Format::of(path).ok_or_else(|| ConfigError::UnknownFormat { path: shown() })?;
        let text = std::fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
            path: shown(),
            reason: error.to_string(),
        })?;
        Ok(Self {
            source: Some(path.to_owned()),
            ..Self::from_str(&text, format)?
        })
    }

    /// States one setting over what the text states: `key=value`, where
    /// the key is the setting's keys joined by `.`, as `run.max_turns=5`
    /// is, and the value is read as a YAML scalar, so `5` is a number,
    /// `true` a boolean, `null` or nothing at all no value, `'5'` the text
    /// `5`, and anything else, `-` and `[bash]` among it, the text as it's
    /// written.
    ///
    /// A key the text doesn't hold is added, with the sections on the way
    /// to it, and a key no setting has is refused when the tree is read as
    /// a config, as it is in the text. A number in the key is a place in
    /// the list it follows, counted from 0, as `tools.mcp.0.command` is.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Override`] when the text isn't `key=value`,
    /// when a key of it is empty, when the way to the key passes through a
    /// value or a place a list doesn't have, and when it names a place in a
    /// list where no list is written. The refusal never shows the value,
    /// and a refused override changes nothing.
    pub fn set(&mut self, key_value: &str) -> Result<(), ConfigError> {
        let refuse = |key: Option<&str>, reason: String| ConfigError::Override {
            key: key.map(str::to_owned),
            reason,
        };
        let Some((key, value)) = key_value.split_once('=') else {
            return Err(refuse(
                None,
                "an override is written `key=value`, and this one holds no `=`".to_owned(),
            ));
        };
        let keys: Vec<&str> = key.split('.').collect();
        if keys.iter().any(|key| key.is_empty()) {
            return Err(refuse(
                Some(key),
                "an override's key is the keys on the way to a setting, joined by `.`, and none \
                 of them is empty"
                    .to_owned(),
            ));
        }
        let value = scalar(value);

        // The override is stated on a copy, which takes the tree's place
        // only once it's stated whole, so a refused one changes nothing.
        let mut tree = self.tree.clone();
        let mut node = &mut tree;
        let mut path = KeyPath::default();
        for (place, name) in keys.iter().enumerate() {
            node = match node {
                Value::Object(entries) => {
                    path = path.key(name);
                    let slot = entries.entry((*name).to_owned()).or_insert(Value::Null);
                    match keys.get(place + 1) {
                        // A section made here would hold the number as a
                        // key, and the refusal would be of a value nobody
                        // wrote.
                        Some(next) if slot.is_null() && next.parse::<usize>().is_ok() => {
                            return Err(refuse(
                                Some(key),
                                format!(
                                    "no list is written at {path}, so `{next}` is no place in one"
                                ),
                            ));
                        }
                        Some(_) if slot.is_null() => *slot = Value::Object(Map::new()),
                        _ => {}
                    }
                    slot
                }
                Value::Array(items) => {
                    let length = items.len();
                    let index = name
                        .parse::<usize>()
                        .ok()
                        .filter(|index| *index < length)
                        .ok_or_else(|| {
                            refuse(
                                Some(key),
                                format!(
                                    "{path} is a list of {length}, and `{name}` is no place in it"
                                ),
                            )
                        })?;
                    path = path.index(index);
                    &mut items[index]
                }
                _ => {
                    return Err(refuse(
                        Some(key),
                        format!("{path} holds a value, and a value has no keys"),
                    ));
                }
            };
        }
        *node = value;
        self.tree = tree;
        self.places.retain(|written, _| !written.is_within(&path));
        self.places.insert(path.clone(), Place::Override);
        // The override's value is one no fill saw, so nothing is recorded
        // for it or for what it replaced.
        self.filled = self.filled.without(&path);
        Ok(())
    }

    /// The tree, with every override stated over the text and, after
    /// [`RawConfig::substituted`], every `${VAR}` filled.
    #[must_use]
    pub fn tree(&self) -> &Value {
        &self.tree
    }

    /// The tree with every `${VAR}` and `${VAR:-default}` of every text but
    /// the prompts' filled from `lookup`, as text: a `${NAME}` is what the
    /// variable holds, a `${NAME:-default}` that or the default when the
    /// variable is unset or empty, and `$${` the text `${`. Each key the
    /// fill changed is recorded with the text as written, so every message,
    /// the resolved config and the digest show it as the file wrote it, and
    /// after the fill every `${` left is text.
    ///
    /// A `${VAR}` at `model.api_key` whose variable is unset leaves the key
    /// unset rather than refusing it, since whether a key is needed is the
    /// provider's to say when the config is checked.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Invalid`] for the first text that can't be
    /// filled: a `${` that begins no reference to a variable, a variable
    /// with no default that isn't set, or one that holds what isn't UTF-8.
    /// The refusal names the key and where it was written, shows the text
    /// as written, and shows nothing a variable holds.
    pub fn substituted(
        mut self,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, ConfigError> {
        let mut tree = self.tree.clone();
        let fills = substitute::fill(&mut tree, &lookup).map_err(|unfilled| {
            let key = unfilled.key;
            ConfigError::Invalid {
                value: shown(&key, key.find(&self.tree), &self.filled),
                place: place_in(&self.places, &key),
                key: key.to_string(),
                reason: unfilled.reason,
            }
        })?;
        self.tree = tree;
        self.filled = Filled::new(fills);
        Ok(self)
    }

    /// The config the tree holds.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::UnknownKey`] for a key no setting has,
    /// [`ConfigError::Invalid`] for a value of the wrong kind or one the
    /// setting doesn't take, and for a text that holds a `${` when
    /// [`RawConfig::substituted`] didn't run, and [`ConfigError::Missing`]
    /// for a setting that has no default and isn't stated. Each names the
    /// key and where it was written, and shows the value as the text wrote
    /// it.
    pub fn config(&self) -> Result<Config, ConfigError> {
        if !self.filled.ran()
            && let Some(key) = wants_a_fill(&self.tree, &KeyPath::default())
        {
            return Err(ConfigError::Invalid {
                value: shown(&key, key.find(&self.tree), &self.filled),
                place: place_in(&self.places, &key),
                key: key.to_string(),
                reason: "it was never filled; call .substituted(lookup) on the RawConfig before \
                         .config()"
                    .to_owned(),
            });
        }
        let config = Config::deserialize(At {
            value: &self.tree,
            path: KeyPath::default(),
        })
        .map_err(|fault| self.refusal(fault))?;
        // A credential key written as `null` states nothing, so a value set
        // in code afterwards isn't held to the text's rule; a `${VAR}` that
        // came out unset keeps its place, since the text beside it may be a
        // key.
        let mut places = self.places.clone();
        places.retain(|key, _| {
            !(is_credential(key)
                && key.find(&self.tree) == Some(&Value::Null)
                && self.filled.at(key).is_none())
        });
        Ok(Config {
            source: self.source.clone(),
            places: Places::new(places),
            filled: self.filled.clone(),
            ..config
        })
    }

    fn refusal(&self, fault: Fault) -> ConfigError {
        let key = fault.key.unwrap_or_default();
        let place = place_in(&self.places, &key);
        match fault.kind {
            FaultKind::UnknownKey { known, .. } => ConfigError::UnknownKey {
                key: key.to_string(),
                place,
                known: listed(known),
            },
            FaultKind::Missing { .. } => ConfigError::Missing {
                key: key.to_string(),
                reason: "the setting has no default".to_owned(),
            },
            FaultKind::Refused { reason } => ConfigError::Invalid {
                value: shown(&key, key.find(&self.tree), &self.filled),
                key: key.to_string(),
                place,
                reason,
            },
        }
    }
}

/// The first key of `value`, under `key`, whose text wants a fill: one that
/// holds `${`, in any setting but a prompt, which no fill reads.
fn wants_a_fill(value: &Value, key: &KeyPath) -> Option<KeyPath> {
    match value {
        Value::String(text) => {
            (substitute::wants_a_fill(text) && !substitute::is_unfilled(key)).then(|| key.clone())
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| wants_a_fill(item, &key.index(index))),
        Value::Object(entries) => entries
            .iter()
            .find_map(|(name, item)| wants_a_fill(item, &key.key(name))),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

/// A value of a YAML document, with where each key and item was written.
#[derive(Debug)]
enum Node {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
    List(Vec<Spanned<Node>>),
    Mapping(Vec<(Spanned<String>, Spanned<Node>)>),
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a YAML value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Node, E> {
        Ok(Node::Bool(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Node, E> {
        Ok(Node::Unsigned(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Node, E> {
        Ok(Node::Signed(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Node, E> {
        Ok(Node::Float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Node, E> {
        Ok(Node::Text(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Node, E> {
        Ok(Node::Text(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Node, A::Error> {
        let mut list = Vec::new();
        while let Some(item) = items.next_element()? {
            list.push(item);
        }
        Ok(Node::List(list))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Node, A::Error> {
        let mut mapping = Vec::new();
        while let Some(key) = entries.next_key()? {
            mapping.push((key, entries.next_value()?));
        }
        Ok(Node::Mapping(mapping))
    }
}

/// The options every YAML config is read with.
fn yaml_options() -> serde_saphyr::Options {
    // Older YAML reads `yes` and `no` as booleans, and a system prompt that
    // says `yes` would be refused for holding one.
    serde_saphyr::options! {
        strict_booleans: true,
        with_snippet: false,
    }
}

fn line_of<T>(spanned: &Spanned<T>) -> Place {
    Place::Line(u32::try_from(spanned.referenced.line()).unwrap_or(u32::MAX))
}

fn yaml(text: &str) -> Result<(Value, BTreeMap<KeyPath, Place>), String> {
    let node: Spanned<Node> = serde_saphyr::from_str_with_options(text, yaml_options())
        .map_err(|error| error.to_string())?;
    let mut places = BTreeMap::new();
    let tree = tree_of(node.value, &KeyPath::default(), &mut places);
    Ok((tree, places))
}

fn tree_of(node: Node, path: &KeyPath, places: &mut BTreeMap<KeyPath, Place>) -> Value {
    match node {
        Node::Null => Value::Null,
        Node::Bool(value) => Value::Bool(value),
        Node::Unsigned(value) => Value::Number(value.into()),
        Node::Signed(value) => Value::Number(value.into()),
        // The reader refuses a float that isn't finite, and every finite
        // one is a JSON number.
        Node::Float(value) => Number::from_f64(value).map_or(Value::Null, Value::Number),
        Node::Text(value) => Value::String(value),
        Node::List(items) => Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let path = path.index(index);
                    places.insert(path.clone(), line_of(&item));
                    tree_of(item.value, &path, places)
                })
                .collect(),
        ),
        Node::Mapping(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| {
                    let path = path.key(&key.value);
                    places.insert(path.clone(), line_of(&key));
                    let value = tree_of(value.value, &path, places);
                    (key.value, value)
                })
                .collect(),
        ),
    }
}

/// The value an override gives: what YAML reads it as when that's a
/// scalar, and otherwise the text as it's written, since an override
/// states one value and `-` or `a: b` would be a list or a mapping.
fn scalar(text: &str) -> Value {
    match serde_saphyr::from_str_with_options::<Spanned<Node>>(text, yaml_options()) {
        Ok(Spanned {
            value:
                node @ (Node::Null
                | Node::Bool(_)
                | Node::Unsigned(_)
                | Node::Signed(_)
                | Node::Float(_)
                | Node::Text(_)),
            ..
        }) => tree_of(node, &KeyPath::default(), &mut BTreeMap::new()),
        _ => Value::String(text.to_owned()),
    }
}

fn json(text: &str) -> Result<(Value, BTreeMap<KeyPath, Place>), String> {
    let tree: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut scan = Scan {
        text,
        at: 0,
        line: 1,
        places: BTreeMap::new(),
    };
    scan.value(&KeyPath::default())?;
    Ok((tree, scan.places))
}

/// A walk over JSON text that the JSON reader has already read whole,
/// which notes the line of every key and item, and refuses a key written
/// twice in one object, where the reader keeps the last.
struct Scan<'a> {
    text: &'a str,
    at: usize,
    line: u32,
    places: BTreeMap<KeyPath, Place>,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn blank(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b'\n' => self.line += 1,
                b' ' | b'\t' | b'\r' => {}
                _ => return,
            }
            self.at += 1;
        }
    }

    fn value(&mut self, path: &KeyPath) -> Result<(), String> {
        self.blank();
        match self.peek() {
            Some(b'{') => self.object(path),
            Some(b'[') => self.array(path),
            Some(b'"') => self.string().map(drop),
            _ => {
                while self
                    .peek()
                    .is_some_and(|byte| !b",]} \t\r\n".contains(&byte))
                {
                    self.at += 1;
                }
                Ok(())
            }
        }
    }

    fn object(&mut self, path: &KeyPath) -> Result<(), String> {
        self.at += 1;
        let mut keys = BTreeSet::new();
        loop {
            self.blank();
            match self.peek() {
                Some(b'"') => {
                    let line = self.line;
                    let key = self.string()?;
                    if !keys.insert(key.clone()) {
                        return Err(format!("the key `{key}` is written twice, on line {line}"));
                    }
                    let path = path.key(&key);
                    self.places.insert(path.clone(), Place::Line(line));
                    self.blank();
                    // The colon between a key and its value.
                    self.at += 1;
                    self.value(&path)?;
                }
                Some(b',') => self.at += 1,
                _ => {
                    self.at += 1;
                    return Ok(());
                }
            }
        }
    }

    fn array(&mut self, path: &KeyPath) -> Result<(), String> {
        self.at += 1;
        let mut index = 0;
        loop {
            self.blank();
            match self.peek() {
                Some(b']') | None => {
                    self.at += 1;
                    return Ok(());
                }
                Some(b',') => self.at += 1,
                Some(_) => {
                    let path = path.index(index);
                    self.places.insert(path.clone(), Place::Line(self.line));
                    self.value(&path)?;
                    index += 1;
                }
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.at;
        self.at += 1;
        while let Some(byte) = self.peek() {
            self.at += if byte == b'\\' { 2 } else { 1 };
            if byte == b'"' {
                break;
            }
        }
        serde_json::from_str(&self.text[start..self.at]).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
