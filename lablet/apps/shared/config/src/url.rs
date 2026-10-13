//! Where a URL's user information is, which the config's resolved form leaves
//! out and the secrets cut.

use super::substitute;

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
    let after_scheme = match url.split_once("://") {
        Some((scheme, rest)) if !scheme.contains(['/', '?', '#', '@']) => rest,
        _ => url,
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let at = authority.rfind('@')?;
    Some(&authority[..at])
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

#[cfg(test)]
mod tests;
