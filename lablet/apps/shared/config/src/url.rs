//! Where a URL's user information is, which the config's resolved form leaves
//! out and the secrets cut.

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

#[cfg(test)]
mod tests;
