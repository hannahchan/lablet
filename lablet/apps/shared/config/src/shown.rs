//! The mask: what a message, the resolved config, a `Debug` form or the
//! digest shows of a setting, decided in one place from the record of what
//! the fill changed.
//!
//! A credential's value is never shown, wherever it came from; a text the
//! fill changed is shown as the file wrote it; a value holding `@`, or a
//! URL holding `?`, is never shown, since user information and a query may
//! hold a credential and where either ends can't be told. A section or a
//! list is shown under the rule of each setting in it, and an item of a list
//! the config keeps as a set is told by the text the fill gave it, since the
//! set holds its items in its own order and not the file's.

use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use super::key::KeyPath;
use super::secret::Secret;
use super::substitute::{Fill, Filled};

/// The longest a message shows of a value, in characters.
pub(crate) const SHOWN_CHARS: usize = 120;

/// Whether `key` is a credential's: `model.api_key`, a value under a
/// `headers` map or under `tools.builtin.credentials`.
#[must_use]
pub fn is_credential(key: &KeyPath) -> bool {
    key.is_within(&KeyPath::of("model.api_key"))
        || key.is_within(&KeyPath::of("tools.builtin.credentials"))
        || key.is_within(&KeyPath::of("telemetry.otlp.headers"))
        || key.to_string().contains("].headers")
}

/// Whether `key` is a URL whose user information is a credential:
/// `model.base_url`, `telemetry.otlp.endpoint` or an HTTP server's `url`.
fn is_url(key: &KeyPath) -> bool {
    let shown = key.to_string();
    matches!(shown.as_str(), "model.base_url" | "telemetry.otlp.endpoint")
        || (shown.starts_with("tools.mcp[") && shown.ends_with("].url"))
}

/// Whether `key` holds a credential the config writes as one `${VAR}` and
/// nothing else: the key and each of a tool's credentials, where a header
/// is written with text around its `${VAR}`, as `Bearer ${T}` is.
fn is_one_variable_alone(key: &KeyPath) -> bool {
    key.is_within(&KeyPath::of("model.api_key"))
        || key.is_within(&KeyPath::of("tools.builtin.credentials"))
}

/// Whether `text` may hold a credential: user information before an `@`,
/// or a URL's query.
fn may_hold_a_credential(text: &str) -> bool {
    text.contains('@') || (text.contains("://") && text.contains('?'))
}

/// A value as a message shows it, which is as JSON writes it, cut to
/// 120 characters: for a text `filled` records, as the file wrote it; for a
/// credential's key, the `${VAR}` the file wrote or the marker a secret set
/// in code is written as, and nothing else, where the key or a tool's
/// credential written beside its `${VAR}` shows nothing, since the text
/// beside it may be the value; nothing for a value that may hold a
/// credential; and for a section or a list, each setting in it under its
/// own rule, and nothing when one of them shows nothing.
#[must_use]
pub fn shown(key: &KeyPath, value: Option<&Value>, filled: &Filled) -> Option<String> {
    let written = shown_whole(key, value?, filled)?.to_string();
    Some(match written.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{}…", &written[..cut]),
        None => written,
    })
}

/// The value a message may show at `key`, whole: [`shown`] before the cut.
pub(crate) fn shown_whole(key: &KeyPath, value: &Value, filled: &Filled) -> Option<Value> {
    if is_credential(key) {
        return match filled.at(key) {
            Some(fill) if fill.is_one_variable() || !is_one_variable_alone(key) => {
                Some(Value::String(fill.written.clone()))
            }
            Some(_) => None,
            // Nothing is there to show: no key, or a map of no credentials.
            None if value.is_null() || value.as_object().is_some_and(Map::is_empty) => {
                Some(value.clone())
            }
            None => (value.as_str() == Some(Secret::MARKER)).then(|| value.clone()),
        };
    }
    // A text the fill changed is shown as written, whatever the setting
    // reads it into: `telemetry.resource` is written as text and held as a
    // map, whose entries the record doesn't name.
    if let Some(fill) = filled.at(key)
        && !matches!(
            value,
            Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_)
        )
    {
        return (!may_hold_a_credential(&fill.written))
            .then(|| Value::String(fill.written.clone()));
    }
    match value {
        Value::Object(entries) => entries
            .iter()
            .map(|(name, item)| Some((name.clone(), shown_whole(&key.key(name), item, filled)?)))
            .collect::<Option<Map<String, Value>>>()
            .map(Value::Object),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| match item {
                Value::String(text) => match written_item(filled, key, index, text) {
                    Some(written) => Some(Value::String(written)),
                    None => (!may_hold_a_credential(text)).then(|| item.clone()),
                },
                _ => shown_whole(&key.index(index), item, filled),
            })
            .collect::<Option<Vec<Value>>>()
            .map(Value::Array),
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {
            let written = match filled.at(key) {
                Some(fill) => Value::String(fill.written.clone()),
                None => value.clone(),
            };
            if let Value::String(text) = &written
                && may_hold_a_credential(text)
            {
                return None;
            }
            Some(written)
        }
    }
}

/// The text the file wrote for the item of the list at `list` that the fill
/// left as `text`, when the record has one: the fill of the item at `index`
/// when it gave that text, else the fill of any item of the list that did,
/// since a list the config keeps as a set holds its items in its own order
/// and not the file's. An item that matches no fill is shown as it is.
fn written_item(filled: &Filled, list: &KeyPath, index: usize, text: &str) -> Option<String> {
    let gave = |fill: &&Fill| fill.filled == text;
    filled
        .at(&list.index(index))
        .filter(gave)
        .or_else(|| {
            filled
                .iter()
                .filter(|(key, _)| key.is_item_of(list))
                .map(|(_, fill)| fill)
                .find(gave)
        })
        .map(|fill| fill.written.clone())
}

/// Which of the record's keys a serialisation writes back as the file wrote
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenBack {
    /// Every key the fill changed, so nothing a variable gave is written:
    /// what the resolved config and every `Debug` form show.
    Every,
    /// The keys whose setting says where something is rather than what a
    /// run does, a path or a variable a command starts with: what the
    /// digest takes as written, so one file has one digest on every
    /// machine.
    AsWritten,
}

impl WrittenBack {
    fn applies(self, key: &KeyPath) -> bool {
        match self {
            Self::Every => true,
            Self::AsWritten => digested_as_written(key),
        }
    }
}

/// Whether `key` is one the digest takes as the file wrote it: a path, a
/// variable a command starts with, or what starts a server.
fn digested_as_written(key: &KeyPath) -> bool {
    const PREFIXES: [&str; 6] = [
        "model.script",
        "run.transcript_path",
        "prompt.system_file",
        "prompt.skills",
        "tools.builtin.root",
        "tools.builtin.env",
    ];
    let shown = key.to_string();
    if PREFIXES.iter().any(|prefix| within(&shown, prefix)) {
        return true;
    }
    let Some(rest) = shown.strip_prefix("tools.mcp[") else {
        return false;
    };
    let Some((index, field)) = rest.split_once("].") else {
        return false;
    };
    index.bytes().all(|byte| byte.is_ascii_digit())
        && ["command", "args", "env"]
            .iter()
            .any(|name| within(field, name))
}

/// Whether the key written `shown` is `prefix` or a setting inside it.
fn within(shown: &str, prefix: &str) -> bool {
    shown
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
}

/// What says the comment a setting gets after its value, when it gets one.
pub type Comments<'a> = &'a dyn Fn(&KeyPath) -> Option<String>;

/// `value` serialised through the mask: each key `filled` records within
/// `mode` is written as the file wrote it, in place of what the fill gave
/// it, and everything else as `value` writes itself. With `comments`, each
/// setting they name is written through `serde_saphyr`'s `Commented`, which
/// that format writes as a YAML comment after the value and another as a
/// pair, so comments are for the YAML `check --resolved` prints alone.
pub struct Masked<'a, T: ?Sized> {
    pub(crate) value: &'a T,
    pub(crate) filled: &'a Filled,
    pub(crate) mode: WrittenBack,
    comments: Option<Comments<'a>>,
}

impl<'a, T: ?Sized> Masked<'a, T> {
    /// `value` under the mask of `filled`, writing back `mode`'s keys.
    pub fn new(value: &'a T, filled: &'a Filled, mode: WrittenBack) -> Self {
        Self {
            value,
            filled,
            mode,
            comments: None,
        }
    }

    /// [`Masked::new`], with a comment after each setting `comments` names
    /// one for.
    pub fn commented(
        value: &'a T,
        filled: &'a Filled,
        mode: WrittenBack,
        comments: Comments<'a>,
    ) -> Self {
        Self {
            value,
            filled,
            mode,
            comments: Some(comments),
        }
    }
}

impl<T: Serialize + ?Sized> Serialize for Masked<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(Masking {
            inner: serializer,
            at: At {
                path: KeyPath::default(),
                filled: self.filled,
                mode: self.mode,
                comments: self.comments,
            },
        })
    }
}

/// Where a serialisation is in the tree, and what's written back.
#[derive(Clone)]
struct At<'a> {
    path: KeyPath,
    filled: &'a Filled,
    mode: WrittenBack,
    comments: Option<Comments<'a>>,
}

/// `value` with the comment `comment` after it, or as it is when there's
/// none: `serde_saphyr`'s wrapper is a tuple struct it knows by name.
fn with_comment<S: Serializer, T: Serialize + ?Sized>(
    serializer: S,
    value: &T,
    comment: Option<String>,
) -> Result<S::Ok, S::Error> {
    match comment {
        Some(comment) => serde_saphyr::Commented(value, comment).serialize(serializer),
        None => value.serialize(serializer),
    }
}

/// A value and its comment, serialised together, for a field or an entry
/// that takes the value by reference.
struct Said<'a, T: ?Sized> {
    value: &'a T,
    comment: Option<String>,
}

impl<T: Serialize + ?Sized> Serialize for Said<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        with_comment(serializer, self.value, self.comment.clone())
    }
}

impl At<'_> {
    /// The comment the setting at this path gets, when it gets one.
    fn comment(&self) -> Option<String> {
        self.comments.and_then(|comments| comments(&self.path))
    }

    /// The written text to serialise at `key` in place of the value there,
    /// when the record has one that's written back.
    fn written_back(&self, key: &KeyPath) -> Option<&str> {
        if !self.mode.applies(key) {
            return None;
        }
        self.filled.at(key).map(|fill| fill.written.as_str())
    }

    /// The written text to serialise in place of `value`, item `index` of
    /// the list at `list`, which this is the path of, when the record has
    /// one that's written back: a text the fill gave an item of the list,
    /// found by the text, since the list may be kept as a set.
    fn written_back_item<T: Serialize + ?Sized>(
        &self,
        list: &KeyPath,
        index: usize,
        value: &T,
    ) -> Option<String> {
        if !self.mode.applies(&self.path)
            || !self.filled.iter().any(|(key, _)| key.is_item_of(list))
        {
            return None;
        }
        let item = serde_json::to_value(value).ok()?;
        written_item(self.filled, list, index, item.as_str()?)
    }

    /// Whether the record has an entry of the map at this path.
    fn has_entries(&self) -> bool {
        self.filled
            .iter()
            .any(|(key, _)| key.is_entry_of(&self.path))
    }

    fn key(&self, name: &str) -> Self {
        At {
            path: self.path.key(name),
            filled: self.filled,
            mode: self.mode,
            comments: self.comments,
        }
    }

    fn index(&self, index: usize) -> Self {
        At {
            path: self.path.index(index),
            filled: self.filled,
            mode: self.mode,
            comments: self.comments,
        }
    }
}

/// A serialiser that hands everything to `inner` and, at a key the record
/// writes back, the written text in place of the value.
struct Masking<'a, S> {
    inner: S,
    at: At<'a>,
}

/// `value` serialised at `at`, through the mask.
struct Within<'a, T: ?Sized> {
    value: &'a T,
    at: At<'a>,
}

impl<T: Serialize + ?Sized> Serialize for Within<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(Masking {
            inner: serializer,
            at: self.at.clone(),
        })
    }
}

macro_rules! forward {
    ($($method:ident($type:ty)),* $(,)?) => {$(
        fn $method(self, value: $type) -> Result<S::Ok, S::Error> {
            self.inner.$method(value)
        }
    )*};
}

impl<'a, S: Serializer> Serializer for Masking<'a, S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Elements<'a, S::SerializeSeq>;
    type SerializeTuple = Elements<'a, S::SerializeTuple>;
    type SerializeTupleStruct = Elements<'a, S::SerializeTupleStruct>;
    type SerializeTupleVariant = Elements<'a, S::SerializeTupleVariant>;
    type SerializeMap = Entries<'a, S::SerializeMap>;
    type SerializeStruct = Fields<'a, S::SerializeStruct>;
    type SerializeStructVariant = Fields<'a, S::SerializeStructVariant>;

    forward! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32),
        serialize_i64(i64), serialize_i128(i128), serialize_u8(u8), serialize_u16(u16),
        serialize_u32(u32), serialize_u64(u64), serialize_u128(u128), serialize_f32(f32),
        serialize_f64(f64), serialize_char(char), serialize_bytes(&[u8]),
    }

    fn serialize_str(self, value: &str) -> Result<S::Ok, S::Error> {
        // A URL's user information the fill didn't give was set in code,
        // and is written as a secret set in code is: a filled one is
        // written back as the file wrote it before it gets here.
        if is_url(&self.at.path) {
            // An `@` past the authority may end a user or password, as a
            // `\` or a `/` in one puts it there, and where that ends can't
            // be told, whatever user information a reading finds before it,
            // so nothing after the scheme is written.
            if crate::url::holds_an_unread_at(value) {
                let scheme = crate::url::before_authority(value);
                return self
                    .inner
                    .serialize_str(&format!("{scheme}{}", Secret::MARKER));
            }
            if let Some(information) = crate::url::user_information(value) {
                let masked = value.replacen(
                    &format!("{information}@"),
                    &format!("{}@", Secret::MARKER),
                    1,
                );
                return self.inner.serialize_str(&masked);
            }
        }
        self.inner.serialize_str(value)
    }

    fn serialize_none(self) -> Result<S::Ok, S::Error> {
        self.inner.serialize_none()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<S::Ok, S::Error> {
        let at = self.at;
        self.inner.serialize_some(&Within { value, at })
    }

    fn serialize_unit(self) -> Result<S::Ok, S::Error> {
        self.inner.serialize_unit()
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<S::Ok, S::Error> {
        self.inner.serialize_unit_struct(name)
    }

    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<S::Ok, S::Error> {
        self.inner.serialize_unit_variant(name, index, variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<S::Ok, S::Error> {
        let at = self.at;
        self.inner
            .serialize_newtype_struct(name, &Within { value, at })
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<S::Ok, S::Error> {
        let at = self.at.key(variant);
        self.inner
            .serialize_newtype_variant(name, index, variant, &Within { value, at })
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, S::Error> {
        Ok(Elements {
            inner: self.inner.serialize_seq(len)?,
            at: self.at,
            index: 0,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, S::Error> {
        Ok(Elements {
            inner: self.inner.serialize_tuple(len)?,
            at: self.at,
            index: 0,
        })
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, S::Error> {
        Ok(Elements {
            inner: self.inner.serialize_tuple_struct(name, len)?,
            at: self.at,
            index: 0,
        })
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, S::Error> {
        Ok(Elements {
            inner: self
                .inner
                .serialize_tuple_variant(name, index, variant, len)?,
            at: self.at.key(variant),
            index: 0,
        })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, S::Error> {
        // A map the record has entries of may hold none of them and still be
        // written some at its end, so it isn't told it's empty: YAML would
        // begin it as `{}`.
        let len = len.filter(|len| *len > 0 || !self.at.has_entries());
        Ok(Entries {
            inner: self.inner.serialize_map(len)?,
            at: self.at,
            key: None,
            seen: Vec::new(),
        })
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, S::Error> {
        Ok(Fields {
            inner: self.inner.serialize_struct(name, len)?,
            at: self.at,
        })
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, S::Error> {
        Ok(Fields {
            inner: self
                .inner
                .serialize_struct_variant(name, index, variant, len)?,
            at: self.at.key(variant),
        })
    }
}

/// The items of a list, each at its index, where a text the fill gave an
/// item is written back by the text, since the list may be kept as a set.
struct Elements<'a, S> {
    inner: S,
    at: At<'a>,
    index: usize,
}

macro_rules! elements {
    ($trait:ident, $method:ident) => {
        impl<S: $trait> $trait for Elements<'_, S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), S::Error> {
                let index = self.index;
                self.index += 1;
                let at = self.at.index(index);
                let comment = at.comment();
                match at.written_back_item(&self.at.path, index, value) {
                    Some(written) => self.inner.$method(&Said {
                        value: &written,
                        comment,
                    }),
                    None => self.inner.$method(&Said {
                        value: &Within { value, at },
                        comment,
                    }),
                }
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.inner.end()
            }
        }
    };
}

elements!(SerializeSeq, serialize_element);
elements!(SerializeTuple, serialize_element);
elements!(SerializeTupleStruct, serialize_field);
elements!(SerializeTupleVariant, serialize_field);

/// The entries of a map, each at its key. A key is text in every map a
/// config holds; one that isn't is serialised as it is, at no key of the
/// record's. An entry whose `${VAR}` came out empty isn't stated, so the map
/// has none for it, and it's written back after the map's own as the file
/// wrote it: the written form and the digest are then the file's whether or
/// not the variable is set, a credential's name among them.
struct Entries<'a, S> {
    inner: S,
    at: At<'a>,
    /// The key last serialised, for the value that follows it.
    key: Option<String>,
    /// Every key serialised, so the record's entries the map has none for
    /// are told at its end.
    seen: Vec<String>,
}

impl<S: SerializeMap> SerializeMap for Entries<'_, S> {
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), S::Error> {
        self.key = serde_json::to_value(key)
            .ok()
            .and_then(|key| key.as_str().map(str::to_owned));
        self.seen.extend(self.key.clone());
        self.inner.serialize_key(key)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), S::Error> {
        match self.key.take() {
            Some(key) => {
                let at = self.at.key(&key);
                let comment = at.comment();
                match at.written_back(&at.path) {
                    Some(written) => self.inner.serialize_value(&Said {
                        value: written,
                        comment,
                    }),
                    None => self.inner.serialize_value(&Said {
                        value: &Within { value, at },
                        comment,
                    }),
                }
            }
            None => self.inner.serialize_value(value),
        }
    }

    fn end(mut self) -> Result<S::Ok, S::Error> {
        let unstated: Vec<(String, String)> = self
            .at
            .filled
            .iter()
            .filter(|(key, _)| key.is_entry_of(&self.at.path))
            .filter_map(|(key, fill)| {
                let name = key.last_key()?;
                (!self.seen.iter().any(|seen| seen == name))
                    .then(|| (name.to_owned(), fill.written.clone()))
            })
            .collect();
        for (name, written) in unstated {
            let comment = self.at.key(&name).comment();
            self.inner.serialize_entry(
                &name,
                &Said {
                    value: written.as_str(),
                    comment,
                },
            )?;
        }
        self.inner.end()
    }
}

/// The fields of a struct, each at its name.
struct Fields<'a, S> {
    inner: S,
    at: At<'a>,
}

macro_rules! fields {
    ($trait:ident) => {
        impl<S: $trait> $trait for Fields<'_, S> {
            type Ok = S::Ok;
            type Error = S::Error;

            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), S::Error> {
                let at = self.at.key(key);
                let comment = at.comment();
                match at.written_back(&at.path) {
                    Some(written) => self.inner.serialize_field(
                        key,
                        &Said {
                            value: written,
                            comment,
                        },
                    ),
                    None => self.inner.serialize_field(
                        key,
                        &Said {
                            value: &Within { value, at },
                            comment,
                        },
                    ),
                }
            }

            fn skip_field(&mut self, key: &'static str) -> Result<(), S::Error> {
                self.inner.skip_field(key)
            }

            fn end(self) -> Result<S::Ok, S::Error> {
                self.inner.end()
            }
        }
    };
}

fields!(SerializeStruct);
fields!(SerializeStructVariant);

/// `value` as a tree, through the mask of `filled` writing back `mode`'s
/// keys, with every text that may hold a credential hidden: what a `Debug`
/// form shows.
pub(crate) fn debug_tree<T: Serialize>(value: &T, filled: &Filled) -> Value {
    // Nothing of a config fails to serialise: a path is written as it's
    // shown, and a number JSON can't hold is written as `null`.
    let mut tree =
        serde_json::to_value(Masked::new(value, filled, WrittenBack::Every)).unwrap_or_default();
    hide(&mut tree);
    tree
}

/// [`debug_tree`] of a raw tree, with every value at or under a
/// credential's key written as the marker: before the fill the tree holds
/// what the text wrote, a credential written out among it, and after it
/// what each variable held, and neither is typed as a secret yet.
pub(crate) fn debug_raw_tree(tree: &Value, filled: &Filled) -> Value {
    let mut shown = debug_tree(tree, filled);
    mark_credentials(&mut shown, &KeyPath::default());
    shown
}

/// Writes every value at or under a credential's key in `value`, which is
/// at `key`, as the marker.
fn mark_credentials(value: &mut Value, key: &KeyPath) {
    match value {
        Value::Object(entries) => {
            for (name, item) in entries.iter_mut() {
                mark_credentials(item, &key.key(name));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                mark_credentials(item, &key.index(index));
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Number(_) if is_credential(key) => {
            *value = Value::String(Secret::MARKER.to_owned());
        }
        Value::Null | Value::String(_) | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Hides every text of `tree` that may hold a credential.
fn hide(value: &mut Value) {
    match value {
        Value::String(text) if may_hold_a_credential(text) => {
            "..".clone_into(text);
        }
        Value::Array(items) => items.iter_mut().for_each(hide),
        Value::Object(entries) => entries.values_mut().for_each(hide),
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests;
