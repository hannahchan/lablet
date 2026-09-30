//! The digest a run's config is grouped by.

/// SHA-256 of a run's resolved config, written as 64 lower-case hex digits:
/// what groups the runs made from one config.
///
/// The domain hashes nothing. The composition root takes the digest and
/// hands over its 32 bytes, so every value of this type is a digest's form
/// and no other text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigDigest(String);

/// Why text was refused as a config digest.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} isn't a config digest, which is 64 lower-case hex digits")]
pub struct DigestError {
    /// The refused text.
    pub value: String,
}

impl ConfigDigest {
    /// How many hex digits a digest is written in: two to each of
    /// SHA-256's 32 bytes.
    pub const LEN: usize = 64;

    /// The digest whose bytes are `sha256`, written two lower-case hex
    /// digits to a byte.
    #[must_use]
    pub fn from_sha256(sha256: [u8; 32]) -> Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let hex = sha256
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .map(|nibble| char::from(DIGITS[usize::from(nibble)]))
            .collect();
        Self(hex)
    }

    /// The digest `hex` writes, for a reader of a record that holds one.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError`] for anything but 64 lower-case hex digits.
    pub fn new(hex: impl Into<String>) -> Result<Self, DigestError> {
        let value = hex.into();
        let digits = value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if value.len() == Self::LEN && digits {
            Ok(Self(value))
        } else {
            Err(DigestError { value })
        }
    }

    /// The digest as its 64 hex digits.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<ConfigDigest> for String {
    fn from(digest: ConfigDigest) -> Self {
        digest.0
    }
}

display_as_str!(ConfigDigest);

#[cfg(test)]
mod tests;
