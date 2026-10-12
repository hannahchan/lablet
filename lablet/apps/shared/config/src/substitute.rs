//! `${VAR}`: the value of an environment variable, filled into a setting's
//! text before the config is typed.
//!
//! The fill is [`RawConfig::substituted`](super::RawConfig::substituted)'s,
//! over the raw tree, and it always fills as text: a text takes each
//! `${NAME}` as what the variable holds and each `${NAME:-default}` as the
//! default when the variable is unset or empty, and `$${` as the text `${`.
//! Neither prompt is filled, so no prompt is scanned. What was written is
//! recorded by its key, so every message, the resolved config and the digest
//! can show a setting as the file wrote it, and nothing a variable holds
//! reaches them.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use super::key::KeyPath;
use super::shown::is_credential;

/// The keys the fill leaves as they're written: a prompt is text, whatever
/// it holds, and is never scanned.
const UNFILLED: [&str; 2] = ["prompt.system", "prompt.system_file"];

/// The one key an unset variable doesn't refuse: the provider decides
/// whether a key is needed, so a `${VAR}` there that's unset leaves the
/// setting unset, recorded, for the check to refuse by provider.
const API_KEY: &str = "model.api_key";

/// A variable a text named: its name, and whether it was set to something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// The variable's name.
    pub name: String,
    /// Whether it was set, to anything. An unset one gave its default, or
    /// left the setting unset.
    pub set: bool,
}

/// One text the fill changed: as the file wrote it, and the variables it
/// named, in order.
#[derive(Clone, PartialEq, Eq)]
pub struct Fill {
    /// The text as the config wrote it.
    pub written: String,
    /// Each variable the text named, once each, in order.
    pub variables: Vec<Variable>,
    /// The text as the fill left it, which is how an item of a list the
    /// config keeps as a set is told, since the set holds its items in its
    /// own order and not the file's. No form shows it.
    pub(crate) filled: String,
}

impl fmt::Debug for Fill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fill")
            .field("written", &self.written)
            .field("variables", &self.variables)
            .finish_non_exhaustive()
    }
}

impl Fill {
    /// The name of each variable the text named, once each, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.variables.iter().map(|variable| variable.name.as_str())
    }

    /// Whether the text is one reference and nothing else, as `${VAR}` and
    /// `${VAR:-}` are: the variable is then the whole of the setting.
    #[must_use]
    pub fn is_one_variable(&self) -> bool {
        let references = references(&self.written);
        references.len() == 1 && references[0].whole == self.written
    }

    /// The default of a `${VAR:-default}` the text holds, when the text
    /// holds one with a default written out; `Some("")` for `${VAR:-}`.
    #[must_use]
    pub fn default(&self) -> Option<&str> {
        references(&self.written)
            .into_iter()
            .find_map(|reference| reference.default)
    }
}

/// Whether the fill `RawConfig::substituted` ran, and what it changed, by
/// key. Shared between the config and what's resolved from it, so a clone
/// is cheap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filled {
    ran: bool,
    fills: Arc<BTreeMap<KeyPath, Fill>>,
}

impl Filled {
    /// The record of a fill that ran and changed `fills`.
    pub(crate) fn new(fills: BTreeMap<KeyPath, Fill>) -> Self {
        Self {
            ran: true,
            fills: Arc::new(fills),
        }
    }

    /// Whether the fill ran, so every `${` left in a text is text.
    #[must_use]
    pub fn ran(&self) -> bool {
        self.ran
    }

    /// Whether the fill changed nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fills.is_empty()
    }

    /// What the fill changed at `key`, when it changed the text there.
    #[must_use]
    pub fn at(&self, key: &KeyPath) -> Option<&Fill> {
        self.fills.get(key)
    }

    /// What the fill changed at `key` and at every key within it.
    pub fn within<'a>(&'a self, key: &'a KeyPath) -> impl Iterator<Item = (&'a KeyPath, &'a Fill)> {
        // A path sorts before every path within it, and those come together.
        self.fills
            .range(key..)
            .take_while(move |(at, _)| at.is_within(key))
    }

    /// Every key the fill changed, with what it changed there.
    pub fn iter(&self) -> impl Iterator<Item = (&KeyPath, &Fill)> {
        self.fills.iter()
    }

    /// The record without `key` and the keys within it, after an override
    /// set a value there that no fill saw.
    pub(crate) fn without(&self, key: &KeyPath) -> Self {
        let fills = self
            .fills
            .iter()
            .filter(|(at, _)| !at.is_within(key))
            .map(|(at, fill)| (at.clone(), fill.clone()))
            .collect();
        Self {
            ran: self.ran,
            fills: Arc::new(fills),
        }
    }
}

/// Why a text couldn't be filled, at which key, and the text as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unfilled {
    pub(crate) key: KeyPath,
    pub(crate) reason: String,
}

/// Fills every text of `tree` but the prompts', in place, and records what
/// changed. `lookup` is what each variable holds.
pub(crate) fn fill(
    tree: &mut Value,
    lookup: &dyn Fn(&str) -> Option<OsString>,
) -> Result<BTreeMap<KeyPath, Fill>, Unfilled> {
    let mut fills = BTreeMap::new();
    fill_in(tree, &KeyPath::default(), lookup, &mut fills)?;
    Ok(fills)
}

fn fill_in(
    value: &mut Value,
    key: &KeyPath,
    lookup: &dyn Fn(&str) -> Option<OsString>,
    fills: &mut BTreeMap<KeyPath, Fill>,
) -> Result<(), Unfilled> {
    match value {
        Value::String(text) => {
            if is_unfilled(key) {
                return Ok(());
            }
            let at_api_key = *key == KeyPath::of(API_KEY);
            match replaced(text, lookup, at_api_key).map_err(|reason| Unfilled {
                key: key.clone(),
                reason,
            })? {
                Replaced::Unchanged => {}
                Replaced::Text(filled, variables) => {
                    let written = std::mem::replace(text, filled);
                    // The record keeps a filled text only to match a list
                    // item back to its fill, which no credential is, so a
                    // credential's value isn't copied into it.
                    let filled = if is_credential(key) {
                        String::new()
                    } else {
                        text.clone()
                    };
                    fills.insert(
                        key.clone(),
                        Fill {
                            written,
                            variables,
                            filled,
                        },
                    );
                }
                Replaced::Unset(variables) => {
                    let written = std::mem::take(text);
                    fills.insert(
                        key.clone(),
                        Fill {
                            written,
                            variables,
                            filled: String::new(),
                        },
                    );
                    *value = Value::Null;
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                fill_in(item, &key.index(index), lookup, fills)?;
            }
        }
        Value::Object(entries) => {
            for (name, item) in entries.iter_mut() {
                fill_in(item, &key.key(name), lookup, fills)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

/// What the fill made of one text.
enum Replaced {
    /// The text holds no `$` the fill reads, so it stays as it is.
    Unchanged,
    /// The text with each reference and escape replaced, and the variables
    /// it named.
    Text(String, Vec<Variable>),
    /// The text named an unset variable at the one key that leaves the
    /// setting unset.
    Unset(Vec<Variable>),
}

/// One `${...}` of a text, as written.
struct Reference<'a> {
    /// The reference whole, `${` to `}`.
    whole: &'a str,
    /// What follows `:-`, when the reference has a default.
    default: Option<&'a str>,
}

/// Every reference of `text`, in order; an escape and a `${` that begins no
/// reference are passed over.
fn references(text: &str) -> Vec<Reference<'_>> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        let after = &rest[at + 1..];
        if let Some(beyond) = after.strip_prefix("${") {
            rest = beyond;
        } else if let Some(reference) = after.strip_prefix('{')
            && let Some((inside, beyond)) = reference.split_once('}')
            && let Some((_, default)) = parts(inside)
        {
            found.push(Reference {
                whole: &rest[at..rest.len() - beyond.len()],
                default,
            });
            rest = beyond;
        } else {
            rest = after;
        }
    }
    found
}

/// The name and the default of what's between `${` and `}`, when it's a
/// reference.
fn parts(inside: &str) -> Option<(&str, Option<&str>)> {
    let (name, default) = match inside.split_once(":-") {
        Some((name, default)) => (name, Some(default)),
        None => (inside, None),
    };
    is_name(name).then_some((name, default))
}

/// `text` with each `${NAME}` replaced by what the variable `NAME` holds,
/// each `${NAME:-default}` by that or the default when the variable is
/// unset or empty, and each `$${` by `${`, so a text can hold `${` itself.
/// Any other `$` is text.
///
/// # Errors
///
/// Returns why the text was refused: a `${` that begins no reference to a
/// variable, a variable with no default that isn't set, or one that holds
/// what isn't UTF-8. No reason shows what a variable holds.
fn replaced(
    text: &str,
    lookup: &dyn Fn(&str) -> Option<OsString>,
    at_api_key: bool,
) -> Result<Replaced, String> {
    let mut written = String::with_capacity(text.len());
    let mut variables: Vec<Variable> = Vec::new();
    let mut changed = false;
    let mut unset = false;
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        written.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(beyond) = after.strip_prefix("${") {
            written.push_str("${");
            changed = true;
            rest = beyond;
        } else if let Some(reference) = after.strip_prefix('{') {
            let (name, default, beyond) = reference
                .split_once('}')
                .and_then(|(inside, beyond)| {
                    parts(inside).map(|(name, default)| (name, default, beyond))
                })
                .ok_or_else(|| {
                    "`${` begins a variable, written `${NAME}` with a name of ASCII letters, \
                     digits and `_`; write `$${` for the text `${`"
                        .to_owned()
                })?;
            let value = lookup(name);
            let set = value.is_some();
            let value = match value {
                Some(value) => Some(
                    value
                        .into_string()
                        .map_err(|_| format!("the variable `{name}` holds what isn't UTF-8"))?,
                ),
                None => None,
            };
            match (value, default) {
                (Some(value), None) => written.push_str(&value),
                (Some(value), Some(_)) if !value.is_empty() => written.push_str(&value),
                (_, Some(default)) => written.push_str(default),
                (None, None) if at_api_key => unset = true,
                (None, None) => return Err(format!("the variable `{name}` isn't set")),
            }
            if !variables.iter().any(|variable| variable.name == name) {
                variables.push(Variable {
                    name: name.to_owned(),
                    set,
                });
            }
            changed = true;
            rest = beyond;
        } else {
            written.push('$');
            rest = after;
        }
    }
    written.push_str(rest);
    Ok(if unset {
        Replaced::Unset(variables)
    } else if changed {
        Replaced::Text(written, variables)
    } else {
        Replaced::Unchanged
    })
}

/// Whether `name` is the name of an environment variable: ASCII letters,
/// digits and `_`, beginning with no digit.
fn is_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Whether `text` holds a `${` that a fill would read: a reference, or an
/// escape of one. What `config()` refuses when no fill ran.
pub(crate) fn wants_a_fill(text: &str) -> bool {
    text.contains("${")
}

/// Whether the fill leaves the text at `key` as it's written.
pub(crate) fn is_unfilled(key: &KeyPath) -> bool {
    UNFILLED
        .iter()
        .any(|unfilled| *key == KeyPath::of(unfilled))
}

#[cfg(test)]
mod tests;
