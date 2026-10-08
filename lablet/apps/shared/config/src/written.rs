//! How a duration and a path are written in a config.

/// A duration as text in humantime's syntax, as `500ms` and `10m` are.
///
/// It's written back in one way whatever way it was read in, so `600s` and
/// `10m` come to the same resolved config.
pub mod duration {
    use std::time::Duration;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&humantime::format_duration(*duration))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        // The refusal holds no part of the text, which the message that
        // names the setting shows as it was written.
        let text = String::deserialize(deserializer)?;
        humantime::parse_duration(&text).map_err(|_| {
            D::Error::custom(
                "a duration is a number and a unit, as `500ms`, `10m` and `1h 30m` are",
            )
        })
    }
}

/// A path as the text it's shown as, which every path has: one that isn't
/// UTF-8 is written with U+FFFD where it isn't, so the resolved config of
/// any config can be written.
pub mod path {
    use std::path::{Path, PathBuf};

    use serde::{Serialize, Serializer};

    struct Shown<'a>(&'a Path);

    impl Serialize for Shown<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(&self.0.display())
        }
    }

    #[expect(
        clippy::ref_option,
        reason = "serde hands a serialiser the field by reference, whatever the field holds"
    )]
    pub fn optional<S: Serializer>(
        path: &Option<PathBuf>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        path.as_deref().map(Shown).serialize(serializer)
    }

    pub fn list<S: Serializer>(paths: &[PathBuf], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(paths.iter().map(|path| Shown(path)))
    }
}
