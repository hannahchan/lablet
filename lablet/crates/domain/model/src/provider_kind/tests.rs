use serde_json::json;

use super::*;

#[test]
fn a_provider_kind_prints_what_it_serialises_as() {
    for (kind, spelling) in [
        (ProviderKind::Anthropic, "anthropic"),
        (ProviderKind::Openai, "openai"),
        (ProviderKind::Fake, "fake"),
    ] {
        assert_eq!(kind.to_string(), spelling);
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<ProviderKind>(json!(spelling)).unwrap(),
            kind
        );
    }
}
