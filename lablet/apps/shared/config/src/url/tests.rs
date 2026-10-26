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

/// An `@` past the authority is one no reading takes as user information:
/// a password's `/`, `?` or `#` puts the `@` after it there, written out,
/// in a default or by a variable, and so does an `@` in a path; one that's
/// percent-encoded, or before the authority's end, is read.
#[test]
fn an_at_past_the_authority_is_one_no_reading_takes_as_user_information() {
    for url in [
        "http://user:pa/ss@127.0.0.1:9",
        "http://user:pa?ss@h",
        "http://user:pa#ss@h",
        "http://h/v1/a@b",
        "u:pa/ss@h:4317",
        "http://x/://u:p@h",
    ] {
        assert!(holds_an_unread_at(url), "{url}");
    }
    for url in [
        "http://user:pass@127.0.0.1:9/v1",
        "http://u:p@ss@h/x",
        "http://127.0.0.1:9/v1/a%40b",
        "http://u:pa%2Fss@h",
        "u:p@h:4317",
    ] {
        assert!(!holds_an_unread_at(url), "{url}");
    }
    for written in [
        "http://${U}:${P:-pa/ss}@h",
        "${URL:-http://u:pa/ss@h}",
        "http://${H}/a@b",
    ] {
        assert!(writes_an_unread_at(written), "{written}");
    }
    for written in [
        "http://${U}:${P}@h",
        "${URL}",
        "http://${U}:${P:-pass}@h/v1",
    ] {
        assert!(!writes_an_unread_at(written), "{written}");
    }
}

/// The url crate ends an `http` or `https` authority at a `\` as at a `/`,
/// so a user or password holding one is read as a host and a path there:
/// the `@` after it is one no reading takes as user information, written
/// out, in a default or after a scheme, and `%5C` is read. A `\` past the
/// authority is a path's.
#[test]
fn a_backslash_ends_the_authority_as_the_url_crate_reads_it() {
    for url in [
        "https://CORP\\jdoe:pw@gateway.corp.example/anthropic",
        "https://jdoe:pa\\ss@gateway.corp.example",
        "jdoe:pa\\ss@h:4317",
    ] {
        assert_eq!(user_information(url), None, "{url}");
        assert!(holds_an_unread_at(url), "{url}");
    }
    assert!(writes_an_unread_at("https://${U}:${P:-pa\\ss}@h"));
    assert_eq!(
        user_information("h:4317\\p://u@q"),
        None,
        "a `\\` ends no scheme"
    );

    let encoded = "https://CORP%5Cjdoe:pw@gateway.corp.example/anthropic";
    assert_eq!(user_information(encoded), Some("CORP%5Cjdoe:pw"));
    assert!(!holds_an_unread_at(encoded));
    for url in [
        "https://gateway.corp.example/anthropic\\v1",
        "https://u:p@gateway.corp.example/a\\b",
    ] {
        assert!(!holds_an_unread_at(url), "{url}");
    }
    assert_eq!(
        user_information("https://u:p@gateway.corp.example/a\\b"),
        Some("u:p")
    );
}

/// What follows a URL's user information in its authority is its host and
/// port, as the same reading finds them.
#[test]
fn the_host_and_port_of_a_url_follow_its_user_information() {
    for (url, expected) in [
        ("https://u:p@gw.example:8443/v1", "gw.example:8443"),
        ("https://u:p@ss@gw.example/v1", "gw.example"),
        ("https://gw.example?q=a@b", "gw.example"),
        ("https://[::1]:8443", "[::1]:8443"),
        ("u:p@h:4317", "h:4317"),
        ("https://CORP\\jdoe:pw@gw.example", "CORP"),
        ("${BASE}/v1", "${BASE}"),
    ] {
        assert_eq!(host_and_port(url), expected, "{url}");
    }
}
