//! What a run shows the model before the conversation starts, as the record
//! names it: the digest of the system prompt, and the size and the digest of
//! the tool specs.
//!
//! The loop takes both digests from what it's about to send. A caller that
//! handed them in could pair one with a prompt or a tool set it wasn't taken
//! from, and two runs shown different things would then look the same.

use std::fmt::Write as _;

use lablet_model::ToolSpec;
use sha2::{Digest as _, Sha256};

/// The tool specs a run offers, measured: each as compact JSON in the
/// model's serde form, in the order they're offered, with nothing between
/// them.
///
/// Each spec is serialised once, for the size and the digest alike, so the
/// two are of the same bytes. A spec is a JSON object, which says where it
/// ends, so two lists of specs never come to the same bytes.
pub(crate) struct OfferedSpecs {
    /// How many bytes the specs come to.
    pub(crate) bytes: u64,
    /// SHA-256 of those bytes, in hex.
    pub(crate) digest: String,
}

impl OfferedSpecs {
    pub(crate) fn measure(specs: &[ToolSpec]) -> Self {
        let mut hasher = Sha256::new();
        let mut bytes = 0;
        // A spec always serialises, since the keys of its schema are strings,
        // so the error has no path here and nothing is ever left out.
        for json in specs
            .iter()
            .filter_map(|spec| serde_json::to_vec(spec).ok())
        {
            bytes += json.len() as u64;
            hasher.update(&json);
        }
        Self {
            bytes,
            digest: hex(&hasher.finalize()),
        }
    }
}

/// SHA-256 of the system prompt, in hex.
pub(crate) fn system_prompt_digest(system: &str) -> String {
    hex(&Sha256::digest(system))
}

/// Lower-case hex, two digits to a byte.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        // Writing to a `String` can't fail, so there's no error to report.
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}
