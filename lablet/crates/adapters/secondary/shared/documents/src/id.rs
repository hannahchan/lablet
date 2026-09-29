//! How the domain's identifiers are written and read: each as a bare string,
//! read through the rule the domain holds for it, so a shape that holds one
//! never holds a string that isn't one.

/// Declares the module a field names in `#[serde(with = "...")]`.
macro_rules! bare_string {
    ($module:ident, $id:ty) => {
        pub(crate) mod $module {
            use serde::de::Error as _;
            use serde::{Deserialize as _, Deserializer, Serializer};

            pub(crate) fn serialize<S: Serializer>(
                id: &$id,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(id.as_str())
            }

            pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$id, D::Error> {
                <$id>::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

bare_string!(run_id, lablet_model::RunId);
bare_string!(tool_call_id, lablet_model::ToolCallId);
bare_string!(tool_name, lablet_model::ToolName);

#[cfg(test)]
mod tests;
