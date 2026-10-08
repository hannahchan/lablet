//! Where a setting is in a config: the keys and the list places on the way
//! to it, and the line it was written on.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// The keys and list places from the top of a config to one setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyPath(Vec<Segment>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Segment {
    Key(String),
    Index(usize),
}

impl KeyPath {
    /// The path of a setting that's named by its keys alone, as
    /// `tools.builtin.root` is.
    #[must_use]
    pub fn of(dotted: &str) -> Self {
        Self(
            dotted
                .split('.')
                .map(|key| Segment::Key(key.to_owned()))
                .collect(),
        )
    }

    /// The setting `key` of the section at this path.
    #[must_use]
    pub fn key(&self, key: &str) -> Self {
        let mut path = self.clone();
        path.0.push(Segment::Key(key.to_owned()));
        path
    }

    /// The item `index` of the list at this path.
    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        let mut path = self.clone();
        path.0.push(Segment::Index(index));
        path
    }

    /// Whether this path is `other` or a setting inside it.
    #[must_use]
    pub fn is_within(&self, other: &Self) -> bool {
        self.0.starts_with(&other.0)
    }

    /// What's at this path of `tree`, when anything is.
    #[must_use]
    pub fn find<'a>(&self, tree: &'a serde_json::Value) -> Option<&'a serde_json::Value> {
        self.0.iter().try_fold(tree, |node, segment| match segment {
            Segment::Key(key) => node.get(key),
            Segment::Index(index) => node.get(index),
        })
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (place, segment) in self.0.iter().enumerate() {
            match segment {
                Segment::Key(key) if place == 0 => f.write_str(key)?,
                Segment::Key(key) => write!(f, ".{key}")?,
                Segment::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

/// Where a setting's value was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    /// On this line of the config's text, counted from 1.
    Line(u32),
    /// By an override, after the text was read.
    Override,
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Line(line) => write!(f, "line {line}"),
            Self::Override => f.write_str("an override"),
        }
    }
}

/// Where each setting of a config was written, when it was read from text.
///
/// Two configs that state the same settings are the same config wherever
/// their settings were written, so every `Places` equals every other.
#[derive(Debug, Clone, Default)]
pub struct Places(Option<Arc<BTreeMap<KeyPath, Place>>>);

impl Places {
    pub fn new(places: BTreeMap<KeyPath, Place>) -> Self {
        Self(Some(Arc::new(places)))
    }

    /// Where the setting at `key` was written, as [`place_in`] says; `None`
    /// for a setting the config doesn't state, and for any setting of a
    /// config made in code.
    pub fn of(&self, key: &KeyPath) -> Option<Place> {
        place_in(self.0.as_ref()?, key)
    }
}

/// Where the setting at `key` was written, of `places`: by an override when
/// an override set it or any setting within it, since the value there is
/// then one the override made, and otherwise where the setting was written,
/// when it was.
pub fn place_in(places: &BTreeMap<KeyPath, Place>, key: &KeyPath) -> Option<Place> {
    // A path sorts before every path within it, and those come together.
    let overridden = places
        .range(key..)
        .take_while(|(written, _)| written.is_within(key))
        .any(|(_, place)| *place == Place::Override);
    if overridden {
        Some(Place::Override)
    } else {
        places.get(key).copied()
    }
}

impl PartialEq for Places {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests;
