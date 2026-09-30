use serde_json::json;

use super::*;

const fn spelling(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::Openai => "openai",
        ProviderKind::Fake => "fake",
    }
}

#[test]
fn a_provider_kind_prints_its_spelling_and_is_measured_as_it() {
    for kind in ProviderKind::ALL {
        let spelling = spelling(kind);
        assert_eq!(kind.as_str(), spelling);
        assert_eq!(kind.to_string(), spelling);
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(spelling));
    }
}
