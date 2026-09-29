use std::time::Duration;

use lablet_model::{Answer, OutputCap, OutputCut, ToolCallEnd, ToolCallStatus, ToolSource};

use super::*;

/// The text `pieces` are read as, fed one after another.
fn read(pieces: &[&[u8]]) -> String {
    let mut text = Text::new(None);
    for piece in pieces {
        text.feed(piece);
    }
    whole(text.kept())
}

fn whole(kept: KeptOutput) -> String {
    sent(kept, None).concat()
}

/// The items the model is sent of `kept` under `cap`.
fn sent(kept: KeptOutput, cap: Option<OutputCap>) -> Vec<String> {
    let status = ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok);
    Answer::measured(status, kept, cap, Duration::ZERO, Duration::ZERO)
        .content()
        .iter()
        .map(|lablet_model::ToolResultContent::Text(text)| text.clone())
        .collect()
}

#[test]
fn a_character_that_one_piece_begins_and_the_next_ends_is_read_whole() {
    let euro = "€".as_bytes();

    for cut in 1..euro.len() {
        let (begins, ends) = euro.split_at(cut);
        assert_eq!(read(&[b"5 ", begins, ends, b" each"]), "5 € each");
    }
    assert_eq!(
        read(&[b"5 ", &euro[..1], &euro[1..2], &euro[2..], b" each"]),
        "5 € each",
        "a piece that neither begins nor ends the character"
    );
}

#[test]
fn bytes_that_are_no_character_are_read_as_one_that_says_so() {
    assert_eq!(read(&[b"a\xFFb"]), "a\u{FFFD}b");
    assert_eq!(read(&[b"a\xE2\x82", b"b"]), "a\u{FFFD}b");
    assert_eq!(
        read(&[b"a\xE2", b"\xE2\x82\xAC"]),
        "a\u{FFFD}€",
        "a character begun and never ended, before one that's whole"
    );
}

#[test]
fn pieces_are_read_as_the_whole_would_be() {
    let bytes = b"ok \xF0\x9F\x98\x80 \xF0\x9F\x98 \xC3\xA9\xC3 \x80 end\xE2\x82";
    let expected = String::from_utf8_lossy(bytes);

    for size in 1..bytes.len() {
        let pieces: Vec<&[u8]> = bytes.chunks(size).collect();
        assert_eq!(read(&pieces), expected, "in pieces of {size}");
    }
}

#[test]
fn a_character_the_output_ends_inside_is_no_character() {
    assert_eq!(read(&[b"a\xE2\x82"]), "a\u{FFFD}");
}

#[test]
fn a_line_of_its_own_follows_text_that_ends_a_line_and_text_that_does_not() {
    for (wrote, expected) in [
        ("", "exit code: 0"),
        ("done\n", "done\nexit code: 0"),
        ("done", "done\nexit code: 0"),
        ("done\n\n", "done\n\nexit code: 0"),
    ] {
        let mut text = Text::new(None);
        text.feed(wrote.as_bytes());
        text.line("exit code: 0");
        assert_eq!(whole(text.kept()), expected, "after {wrote:?}");
    }
}

#[test]
fn a_line_follows_a_character_that_was_never_ended() {
    let mut text = Text::new(None);
    text.feed(b"a\xE2");
    text.line("exit code: 0");

    assert_eq!(whole(text.kept()), "a\u{FFFD}\nexit code: 0");
}

#[test]
fn no_more_is_kept_than_the_call_keeps_and_all_of_it_is_counted() {
    let mut text = Text::new(Some(OutputKeep { head: 4, tail: 4 }));
    for _ in 0..1_000 {
        text.feed("€uro ".as_bytes());
    }
    let kept = text.kept();

    assert_eq!(kept.total_bytes(), 7_000);
    assert!(kept.kept_bytes() <= 8, "{} bytes", kept.kept_bytes());
    let cap = OutputCap::new(8, OutputCut::HeadTail).unwrap();
    assert_eq!(
        sent(kept, Some(cap)),
        ["€u", "[truncated: 6992 of 7000 bytes left out]", "uro "]
    );
}
