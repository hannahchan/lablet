//! Where a URL's user information is, which the config's resolved form leaves
//! out and the secrets cut.

use std::ops::Range;

use super::substitute::{self, Fill};

/// The user information of `url`, the `user:password` before the `@` of
/// its authority, when it has one. A short reading of RFC 3986 rather than
/// a URL crate: the authority runs from after the scheme's `://`, or from
/// the start of a value without a scheme, which the gRPC exporter gives
/// one, to the first `/`, `?` or `#`, and the user information is what's
/// before the last `@` in it. A `://` ends a scheme only when no `/`, `?`,
/// `#` or `@` comes before it. User information that holds an unencoded
/// `/`, `?` or `#` isn't found, as a URL parser doesn't find it, so a
/// refusal of an endpoint that holds an `@` shows no value at all.
#[must_use]
pub fn user_information(url: &str) -> Option<&str> {
    user_information_range(url).map(|range| &url[range])
}

/// Where in `url` [`user_information`] is.
fn user_information_range(url: &str) -> Option<Range<usize>> {
    let authority = authority_range(url);
    let at = url[authority.clone()].rfind('@')?;
    Some(authority.start..authority.start + at)
}

/// Where in `url` its authority is, as [`user_information`] reads it.
fn authority_range(url: &str) -> Range<usize> {
    let start = match url.split_once("://") {
        Some((scheme, _)) if !scheme.contains(['/', '?', '#', '@']) => scheme.len() + "://".len(),
        _ => 0,
    };
    let after_scheme = &url[start..];
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    start..start + end
}

/// Whether `url` holds an unencoded `@` past its authority, which no
/// reading of it takes as user information: a user or password holding a
/// `/`, `?` or `#` puts its `@` there, where nothing finds it to cut or
/// leave unshown, as does an `@` in a path.
#[must_use]
pub fn holds_an_unread_at(url: &str) -> bool {
    url[authority_range(url).end..].contains('@')
}

/// [`holds_an_unread_at`] of `written`, a URL as the config wrote it: each
/// `${VAR}` a stand-in, and each default the text it writes.
#[must_use]
pub fn writes_an_unread_at(written: &str) -> bool {
    holds_an_unread_at(&substitute::written_out(written, VARIABLE))
}

/// `url` with its user information and the `@` after it left out.
#[must_use]
pub fn without_user_information(url: &str) -> String {
    match user_information(url) {
        Some(information) => url.replacen(&format!("{information}@"), "", 1),
        None => url.to_owned(),
    }
}

/// What a URL as the config wrote it writes of user information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenInformation {
    /// The written URL has none, so any it holds was given whole by a
    /// variable.
    Absent,
    /// The user and the password are each `${VAR}`s and nothing else, and
    /// the `:` between them.
    Variables,
    /// It writes a user or a password out, in a default among the rest.
    WrittenOut,
}

/// A character no written URL holds, standing for a `${VAR}`: it holds no
/// `@`, `:` or `/`, which would move where user information ends, and no
/// `?` or `#`, which would end a scheme.
const VARIABLE: &str = "\u{0}";

/// What `written`, a URL as the config wrote it, writes of user
/// information: each `${VAR}` a stand-in, and each default the text it
/// writes, since a default is the config's text and not a variable's.
#[must_use]
pub fn written_information(written: &str) -> WrittenInformation {
    let masked = substitute::written_out(written, VARIABLE);
    match user_information(&masked) {
        None => WrittenInformation::Absent,
        Some(information) if matches!(information.replace(VARIABLE, "").as_str(), "" | ":") => {
            WrittenInformation::Variables
        }
        Some(_) => WrittenInformation::WrittenOut,
    }
}

/// A `${VAR}` of a URL that gave it what the URL can't read as the config
/// wrote it, by the variable's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// A variable the written user information holds gave an unencoded
    /// `@`, `/`, `?` or `#`, which would end the user information or the
    /// authority where the text didn't.
    UserInformation(String),
    /// A variable elsewhere in the URL gave an `@`, which may be a user or
    /// password the URL can't read, since what's before it is then read as
    /// one, or isn't when a `/` comes first.
    At(String),
}

/// The first variable, in the order the text names them, that gave the URL
/// `fill` records what it can't read as written: a variable written in its
/// user information, the span before the written `@`, whose value holds an
/// unencoded `@`, `/`, `?` or `#`, or any other whose value holds an `@`.
/// Only what each value held of those is read, from the record: no value
/// is kept to read.
#[must_use]
pub fn unreadable(fill: &Fill) -> Option<Unreadable> {
    let (masked, places) = substitute::placed(&fill.written, VARIABLE);
    let information = user_information_range(&masked);
    places.into_iter().find_map(|(name, place)| {
        let index = fill.names().position(|variable| variable == name)?;
        let held = fill.delimiters.get(index)?;
        let within = information
            .as_ref()
            .is_some_and(|span| span.start <= place.start && place.end <= span.end);
        if within && !held.is_empty() {
            Some(Unreadable::UserInformation(name.to_owned()))
        } else if held.contains('@') {
            Some(Unreadable::At(name.to_owned()))
        } else {
            None
        }
    })
}

/// The name of each `${VAR}` the URL `written` names outside its user
/// information and the `@` after it, once each, in order: those a
/// credential's variable may not be, since what they fill is shown.
#[must_use]
pub fn outside_user_information(written: &str) -> Vec<&str> {
    let (masked, places) = substitute::placed(written, VARIABLE);
    let information = user_information_range(&masked);
    let mut outside: Vec<&str> = Vec::new();
    for (name, place) in places {
        let within = information
            .as_ref()
            .is_some_and(|span| span.start <= place.start && place.end <= span.end);
        if !within && !outside.contains(&name) {
            outside.push(name);
        }
    }
    outside
}

#[cfg(test)]
mod tests;
