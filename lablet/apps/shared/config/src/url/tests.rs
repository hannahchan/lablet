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
