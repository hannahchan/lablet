use proptest::prelude::*;

use super::*;
use crate::ToolResultContent;

/// A value of the shortest length that's cut.
const KEY: &str = "sk-0123456789abc";

/// A value that begins with [`KEY`] and goes on past it.
const LONGER: &str = "sk-0123456789abc-and-more";

/// A value on several lines, as a PEM is, whose first and last lines are
/// too short to be parts and whose middle two are long enough.
const PEM: &str =
    "-----BEGIN-----\nMIIEvQIBADANBgkqhkiG9w0BAQEF\nAbCdEf0123456789AbCdEf012\n-----END-----";

const MARKER: &str = Secrets::MARKER;

fn secrets(values: &[&str]) -> Secrets {
    Secrets::new(values.iter().map(|&value| value.to_owned()))
}

/// Nothing yet of a text that `values` are cut from, and `keep` is kept of.
fn redacting(values: &[&str], keep: Option<OutputKeep>) -> RedactedOutput {
    RedactedOutput::new(Arc::new(secrets(values)), keep)
}

/// The text of what was kept, of which everything was.
fn text(kept: KeptOutput) -> String {
    let (content, _) = kept.cut(None);
    content
        .into_iter()
        .map(|ToolResultContent::Text(text)| text)
        .collect()
}

/// What's kept of `pieces`, pushed one after another, with `values` cut.
fn pushed(values: &[&str], pieces: &[&str]) -> String {
    let mut output = redacting(values, None);
    for piece in pieces {
        output.push(piece);
    }
    text(output.kept())
}

/// What's kept so far, and what's held back.
fn so_far(output: &RedactedOutput) -> (String, &str) {
    (text(output.kept.clone()), output.held.as_str())
}

#[test]
fn a_value_is_cut_wherever_the_text_holds_it() {
    let text = format!("{KEY} at the start, {KEY}{KEY} twice, and at the end {KEY}");

    assert_eq!(
        pushed(&[KEY], &[&text]),
        format!("{MARKER} at the start, {MARKER}{MARKER} twice, and at the end {MARKER}")
    );
}

#[test]
fn with_no_value_to_cut_the_text_is_kept_as_it_came() {
    let text = format!("nothing is cut from {KEY}");

    assert_eq!(pushed(&[], &[&text]), text);
    assert_eq!(pushed(&[KEY], &["nothing to cut"]), "nothing to cut");
    assert_eq!(pushed(&[KEY], &[]), "");
}

#[test]
fn a_value_shorter_than_the_minimum_is_not_cut_and_one_of_the_minimum_is() {
    let short = &KEY[..Secrets::MIN_BYTES - 1];
    assert_eq!(KEY.len(), Secrets::MIN_BYTES);

    assert_eq!(pushed(&[short], &[KEY]), KEY);
    assert_eq!(pushed(&[short, KEY], &[KEY]), MARKER);
    assert_eq!(secrets(&[short]), Secrets::default());
}

#[test]
fn the_whitespace_around_a_value_is_no_part_of_it() {
    let set_with = format!(" {KEY}\n");

    assert_eq!(
        pushed(&[&set_with], &[&format!("{KEY}\n")]),
        format!("{MARKER}\n")
    );
    assert_eq!(
        secrets(&[&format!("  {}  ", &KEY[..Secrets::MIN_BYTES - 1])]),
        Secrets::default(),
        "a value that's short once its whitespace is gone"
    );
}

#[test]
fn each_line_of_a_value_with_several_is_cut_and_so_is_its_escaped_form() {
    let one_line = "MIIEvQIBADANBgkqhkiG9w0BAQEF\n";
    let escaped = "key=\"-----BEGIN-----\\nMIIEvQIBADANBgkqhkiG9w0BAQEF\\nAbCdEf0123456789AbCdEf012\\n-----END-----\"";

    assert_eq!(pushed(&[PEM], &[PEM]), MARKER, "the whole value");
    assert_eq!(pushed(&[PEM], &[one_line]), format!("{MARKER}\n"));
    assert_eq!(pushed(&[PEM], &[escaped]), format!("key=\"{MARKER}\""));
    assert_eq!(
        pushed(&[PEM], &["-----BEGIN-----\n-----END-----\n"]),
        "-----BEGIN-----\n-----END-----\n",
        "a line under the minimum is no part"
    );
    assert_eq!(
        pushed(
            &[" AbCdEf0123456789AbCdEf012 \n"],
            &["AbCdEf0123456789AbCdEf012"]
        ),
        MARKER,
        "a line is registered without the whitespace around it"
    );
}

#[test]
fn what_follows_a_scheme_word_is_cut_as_the_value_is() {
    for scheme in ["Bearer", "Basic", "Token"] {
        let value = format!("{scheme} {LONGER}");

        assert_eq!(pushed(&[&value], &[&value]), MARKER, "{scheme}, whole");
        assert_eq!(
            pushed(&[&value], &[&format!("token={LONGER};")]),
            format!("token={MARKER};"),
            "{scheme}, the remainder"
        );
    }
    let not_a_scheme = format!("bearer {LONGER}");
    assert_eq!(
        pushed(&[&not_a_scheme], &[LONGER]),
        LONGER,
        "a scheme word is written as a header writes it"
    );
    let short_remainder = "Bearer 0123456789";
    assert_eq!(pushed(&[short_remainder], &[short_remainder]), MARKER);
    assert_eq!(
        pushed(&[short_remainder], &["0123456789"]),
        "0123456789",
        "a remainder under the minimum is no part"
    );
}

#[test]
fn secrets_are_shown_by_how_many_there_are_and_a_value_registered_twice_is_held_once() {
    assert_eq!(
        format!("{:?}", secrets(&[KEY, LONGER, "short", KEY])),
        "Secrets { values: 2 }"
    );
    assert_eq!(
        format!("{:?}", secrets(&[KEY, "short", LONGER, KEY])),
        "Secrets { values: 2 }",
        "wherever the copies are"
    );
    assert_eq!(
        format!("{:?}", secrets(&[PEM])),
        "Secrets { values: 4 }",
        "the value, its two long lines and its escaped form"
    );
}

#[test]
fn secrets_with_nothing_to_cut_say_so() {
    assert!(Secrets::default().is_empty());
    assert!(secrets(&["short"]).is_empty());
    assert!(!secrets(&[KEY]).is_empty());
}

#[test]
fn where_two_values_begin_at_one_place_the_longer_is_cut() {
    for values in [[KEY, LONGER], [LONGER, KEY]] {
        assert_eq!(pushed(&values, &[LONGER]), MARKER, "{values:?}");
        assert_eq!(
            pushed(&values, &[&format!("{KEY}!")]),
            format!("{MARKER}!"),
            "{values:?}"
        );
    }
}

/// A value whose first byte is its last, so two copies can share it.
const SHARES_ITS_ENDS: &str = "x0123456789abcdx";

#[test]
fn copies_that_overlap_are_one_cut_that_shows_nothing_of_either() {
    let repeats = "abababababababab";
    assert_eq!(pushed(&[repeats], &["abababababababababab"]), MARKER);
    assert_eq!(
        pushed(&[SHARES_ITS_ENDS], &["x0123456789abcdx0123456789abcdx!"]),
        format!("{MARKER}!")
    );

    let other = "cdef-and-onward!";
    assert_eq!(
        pushed(
            &["0123456789abcdef", other],
            &["0123456789abcdef-and-onward!"]
        ),
        MARKER
    );
    assert_eq!(
        pushed(&[KEY], &[&format!("{KEY}{KEY}")]),
        format!("{MARKER}{MARKER}")
    );
}

#[test]
fn a_cut_that_one_piece_begins_is_stretched_by_a_copy_the_next_one_ends() {
    let mut output = redacting(&[SHARES_ITS_ENDS], None);

    output.push("x0123456789abcdx0123");
    assert_eq!(so_far(&output), (MARKER.to_owned(), "x0123"));
    assert_eq!(output.covered, 1);

    output.push("456789abcdx and on");
    assert_eq!(text(output.kept()), format!("{MARKER} and on"));
}

#[test]
fn a_value_that_pieces_split_is_cut_as_it_is_in_the_whole_text() {
    let whole = format!("before {LONGER} between {KEY} after");
    let expected = format!("before {MARKER} between {MARKER} after");

    for split in 1..whole.len() {
        let (first, second) = whole.split_at(split);
        assert_eq!(
            pushed(&[KEY, LONGER], &[first, second]),
            expected,
            "at {split}"
        );
    }
    let characters: Vec<String> = whole.chars().map(String::from).collect();
    let characters: Vec<&str> = characters.iter().map(String::as_str).collect();
    assert_eq!(pushed(&[KEY, LONGER], &characters), expected);
}

#[test]
fn only_the_end_that_may_begin_a_value_is_held_back() {
    let mut output = redacting(&[KEY], None);

    output.push("key: sk-01");
    assert_eq!(so_far(&output), ("key: ".to_owned(), "sk-01"));

    output.push("2 is no key");
    assert_eq!(so_far(&output), ("key: sk-012 is no key".to_owned(), ""));
}

#[test]
fn a_value_that_ends_a_piece_is_cut_at_once_unless_a_longer_one_may_follow() {
    let mut output = redacting(&[KEY], None);
    output.push(KEY);
    assert_eq!(so_far(&output), (MARKER.to_owned(), ""));

    let mut output = redacting(&[KEY, LONGER], None);
    output.push(KEY);
    assert_eq!(so_far(&output), (String::new(), KEY));
    output.push("-and-more");
    assert_eq!(so_far(&output), (MARKER.to_owned(), ""));
}

#[test]
fn what_was_held_back_is_cut_or_handed_on_once_the_text_ends() {
    let mut output = redacting(&[KEY, LONGER], None);
    output.push("the key ");
    output.push(KEY);
    assert_eq!(text(output.kept()), format!("the key {MARKER}"));

    assert_eq!(pushed(&[KEY], &["ends with sk-0123"]), "ends with sk-0123");
}

/// A text that's all there at once is cut as the same text pushed in
/// pieces is, its end included: what could begin a value there is handed
/// on, since nothing follows.
#[test]
fn a_whole_text_is_cut_as_one_pushed_in_pieces_is_and_its_end_is_handed_on() {
    let secrets = secrets(&[KEY, LONGER]);
    for text in [
        format!("the key {KEY} and {LONGER}, then {KEY}"),
        format!("ends with {KEY}"),
        "ends with sk-0123".to_owned(),
        "no value".to_owned(),
        String::new(),
    ] {
        assert_eq!(
            secrets.redacted(&text),
            pushed(&[KEY, LONGER], &[&text]),
            "{text:?}"
        );
    }
    assert_eq!(
        secrets.redacted(&format!("{KEY} ends with sk-0123")),
        format!("{MARKER} ends with sk-0123")
    );
}

#[test]
fn the_closing_line_follows_what_was_held_back_and_is_cut_as_a_text_of_its_own() {
    let mut output = redacting(&[KEY], None);
    output.push("ends with sk-01");
    output.close(&format!("said {KEY}"));

    assert_eq!(output.held, "");
    assert_eq!(text(output.kept()), format!("ends with sk-01said {MARKER}"));
}

#[test]
fn the_size_and_the_cut_count_the_text_with_the_marker_in_it() {
    let mut output = redacting(&[LONGER], Some(OutputKeep { head: 8, tail: 0 }));
    output.push(&format!("a {LONGER} b"));
    let kept = output.kept();

    assert_eq!(kept.total_bytes(), (MARKER.len() + 4) as u64);
    assert_eq!(kept.kept_bytes(), 8);
    let (content, truncated_from_bytes) = kept.cut(None);
    assert_eq!(truncated_from_bytes, Some(21));
    assert_eq!(
        content[0],
        ToolResultContent::Text(format!("a {}", &MARKER[..6]))
    );
}

/// A value on two lines, each a part of its own, for the property below.
const TWO_LINES: &str = "pem-0123456789abc\npem-abcdef0123456";

proptest! {
    /// Text cut in any pieces is kept as it's kept whole, whatever values
    /// begin with each other, repeat or overlap, and no value, line of one
    /// or escaped form of one is left in it.
    #[test]
    fn pieces_are_cut_as_the_whole_text_is(
        text in "(a|b|sk-0123456789abc|-and-more|x0123456789abcdx|0123456789abcdx|pem-0123456789abc|pem-abcdef0123456|\n|\\\\n| |€){0,40}",
        splits in proptest::collection::vec(0_usize..200, 0..8),
    ) {
        let values = [KEY, LONGER, "abababababababab", SHARES_ITS_ENDS, TWO_LINES];
        let whole = pushed(&values, &[&text]);
        let mut never_left = values.to_vec();
        never_left.extend(TWO_LINES.lines());
        let escaped = TWO_LINES.replace('\n', "\\n");
        never_left.push(&escaped);
        for value in never_left {
            prop_assert!(!whole.contains(value), "{value} is in {whole}");
        }
        let mut at: Vec<usize> = splits
            .into_iter()
            .map(|split| text.floor_char_boundary(split.min(text.len())))
            .collect();
        at.sort_unstable();
        let mut pieces = Vec::new();
        let mut from = 0;
        for split in at {
            pieces.push(&text[from..split]);
            from = split;
        }
        pieces.push(&text[from..]);

        prop_assert_eq!(pushed(&values, &pieces), whole);
    }
}
