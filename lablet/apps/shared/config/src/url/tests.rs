use super::*;

#[test]
fn the_user_information_of_a_url_is_what_stands_before_the_at_of_its_authority() {
    assert_eq!(user_information("https://u:p@h/x"), Some("u:p"));
    assert_eq!(user_information("https://u@h?q=a@b"), Some("u"));
    assert_eq!(user_information("https://u:p%40q@h#a@b"), Some("u:p%40q"));
    assert_eq!(user_information("http://h/x@y"), None);
    assert_eq!(user_information("https://h"), None);
    assert_eq!(user_information("${BASE}/v1"), None);

    assert_eq!(without_user_information("https://u:p@h/x"), "https://h/x");
    assert_eq!(without_user_information("https://h/x"), "https://h/x");
}

/// A value without a scheme starts with its authority, as the gRPC
/// exporter reads it, so its user information is a URL's.
#[test]
fn a_value_without_a_scheme_starts_with_its_authority() {
    assert_eq!(user_information("u:p@h:4317"), Some("u:p"));
    assert_eq!(user_information("no-scheme:u@h"), Some("no-scheme:u"));
    assert_eq!(user_information("u@h/x://y"), Some("u"));
    assert_eq!(
        user_information("u:p@h://x"),
        Some("u:p"),
        "an `@` ends no scheme"
    );
    assert_eq!(user_information("h:4317/x@y"), None);
    assert_eq!(
        user_information("h:4317/p://u@q"),
        None,
        "a `/` ends no scheme"
    );
    assert_eq!(user_information("h:4317?p://u@q"), None);
    assert_eq!(user_information("h:4317#p://u@q"), None);
    assert_eq!(user_information("h:4317"), None);

    assert_eq!(without_user_information("u:p@h:4317"), "h:4317");
}

/// R17's URL form and R63 read the URL as the config wrote it: a `${VAR}`
/// writes nothing out, and a default writes its text, whatever the
/// variable holds.
#[test]
fn a_written_url_writes_a_user_or_password_out_only_in_its_own_text_or_a_default() {
    use WrittenInformation::{Absent, Variables, WrittenOut};
    for (written, expected) in [
        ("https://${U}@gw", Variables),
        ("https://${U}:${P}@gw/v1", Variables),
        ("${U}:${P}@gw:4317", Variables),
        ("https://${U}:${P:-}@gw", Variables),
        ("https://d:pw@gw/v1", WrittenOut),
        ("https://${U}:literalpw@gw", WrittenOut),
        ("https://pre${U}:${P}@gw", WrittenOut),
        ("https://${U}::${P}@gw", WrittenOut),
        ("${GW:-https://d:pw@gw/v1}", WrittenOut),
        ("https://${U}:${P:-pw}@gw", WrittenOut),
        ("${GW}", Absent),
        ("${GW:-}", Absent),
        ("${GW:-https://gw/v1}", Absent),
        ("https://${HOST}/v1", Absent),
        ("https://gw/v1", Absent),
        ("$${U}:${P}@gw", WrittenOut),
    ] {
        assert_eq!(written_information(written), expected, "{written}");
    }
}
