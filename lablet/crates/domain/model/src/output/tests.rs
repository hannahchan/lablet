use super::*;

const SIXTEEN: &str = "0123456789abcdef";

/// Three bytes to the character, so a limit can land inside one.
const EUROS: &str = "\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}";

fn cap(max_bytes: u64, cut: OutputCut) -> OutputCap {
    OutputCap::new(max_bytes, cut).unwrap()
}

const fn keep(head: u64, tail: u64) -> OutputKeep {
    OutputKeep { head, tail }
}

/// `items` fed as an executor feeds them: each as an item of its own, `step`
/// characters at a time.
fn fed_in_steps(keep: Option<OutputKeep>, items: &[&str], step: usize) -> KeptOutput {
    let mut output = KeptOutput::new(keep);
    for item in items {
        output.item();
        let characters: Vec<char> = item.chars().collect();
        for part in characters.chunks(step) {
            output.push(&part.iter().collect::<String>());
        }
    }
    output
}

/// One item, fed a character at a time, so every boundary is met.
fn fed(keep: Option<OutputKeep>, text: &str) -> KeptOutput {
    fed_in_steps(keep, &[text], 1)
}

fn texts(content: &[ToolResultContent]) -> Vec<&str> {
    content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect()
}

/// What the model is sent of `text` when an executor kept what `cap` asks
/// for, and the size the outcome holds.
fn sent(text: &str, cap: OutputCap) -> (Vec<String>, Option<u64>) {
    let (content, truncated_from_bytes) = fed(Some(cap.keeps()), text).cut(Some(cap));
    let content = texts(&content).into_iter().map(str::to_owned).collect();
    (content, truncated_from_bytes)
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|&text| text.to_owned()).collect()
}

#[test]
fn a_preview_longer_than_the_cap_is_refused() {
    let refused = OutputCap::new(10, OutputCut::Preview { bytes: 11 }).unwrap_err();

    assert_eq!(
        refused,
        OutputCapError::PreviewAboveCap {
            bytes: 11,
            max_bytes: 10
        }
    );
    assert_eq!(
        refused.to_string(),
        "a preview of 11 bytes is longer than the output cap of 10 bytes"
    );
}

#[test]
fn a_preview_no_longer_than_the_cap_is_allowed_and_so_is_every_other_cut() {
    for cut in [
        OutputCut::Preview { bytes: 10 },
        OutputCut::Preview { bytes: 9 },
        OutputCut::Preview { bytes: 0 },
        OutputCut::Head,
        OutputCut::HeadTail,
    ] {
        assert_eq!(
            OutputCap::new(10, cut),
            Ok(OutputCap { max_bytes: 10, cut }),
            "{cut:?}"
        );
    }
    assert_eq!(
        OutputCap::new(0, OutputCut::Head),
        Ok(OutputCap {
            max_bytes: 0,
            cut: OutputCut::Head
        })
    );
}

/// A preview of 2,000 bytes under a cap of 50,000 still has 50,000 kept, or
/// an output of 30,000 would arrive cut and be sent as if whole.
#[test]
fn an_executor_keeps_the_cap_s_worth_of_the_start_whatever_the_cut() {
    assert_eq!(
        cap(50_000, OutputCut::Preview { bytes: 2_000 }).keeps(),
        OutputKeep {
            head: 50_000,
            tail: 0
        }
    );
    assert_eq!(
        cap(50_000, OutputCut::Head).keeps(),
        OutputKeep {
            head: 50_000,
            tail: 0
        }
    );
}

#[test]
fn only_a_cut_that_sends_the_end_has_it_kept_and_at_half_the_cap_rounded_down() {
    assert_eq!(
        cap(10, OutputCut::HeadTail).keeps(),
        OutputKeep { head: 10, tail: 5 }
    );
    assert_eq!(
        cap(11, OutputCut::HeadTail).keeps(),
        OutputKeep { head: 11, tail: 5 }
    );
}

#[test]
fn with_no_limit_everything_that_is_fed_is_kept() {
    let output = fed(None, SIXTEEN);

    assert_eq!(texts(&output.content), [SIXTEEN]);
    assert_eq!(output.tail, "");
    assert_eq!(output.total_bytes(), 16);
    assert_eq!(output.kept_bytes(), 16);
}

#[test]
fn text_is_kept_while_the_start_has_room_and_counted_after_that() {
    let mut output = KeptOutput::new(Some(keep(10, 0)));
    output.push("01234567");
    output.push("89abcd");
    output.push("ef");

    assert_eq!(texts(&output.content), ["0123456789"]);
    assert_eq!(output.tail, "");
    assert_eq!(output.total_bytes(), 16);
    assert_eq!(output.kept_bytes(), 10);
}

#[test]
fn parts_join_into_one_item_until_a_new_item_begins() {
    let mut output = KeptOutput::new(None);
    output.push("ab");
    output.push("cd");
    output.item();
    output.push("ef");
    output.push("gh");

    assert_eq!(texts(&output.content), ["abcd", "efgh"]);
    assert_eq!(output.total_bytes(), 8);
}

#[test]
fn an_item_nothing_was_kept_of_is_not_an_item() {
    let mut output = KeptOutput::new(Some(keep(4, 0)));
    output.item();
    output.item();
    output.push("");
    output.item();
    output.push("0123456789");
    output.item();
    output.push("abcdef");
    output.item();

    assert_eq!(texts(&output.content), ["0123"]);
    assert_eq!(output.total_bytes(), 16);
}

#[test]
fn the_start_ends_at_the_last_character_boundary_its_bytes_allow() {
    let output = fed(Some(keep(5, 0)), EUROS);

    assert_eq!(texts(&output.content), ["\u{20ac}"]);
    assert_eq!(output.kept_bytes(), 3);
    assert_eq!(output.total_bytes(), 18);
}

/// The start is the output's own beginning with nothing missing, so text
/// that would fit in the room a character left over isn't kept.
#[test]
fn the_first_byte_left_out_closes_the_start() {
    let mut output = KeptOutput::new(Some(keep(5, 0)));
    output.push("\u{20ac}");
    output.push("\u{20ac}");
    output.push("ab");
    output.item();
    output.push("c");

    assert_eq!(texts(&output.content), ["\u{20ac}"]);
    assert_eq!(output.total_bytes(), 9);
}

#[test]
fn a_start_that_is_exactly_full_keeps_nothing_more() {
    let mut output = KeptOutput::new(Some(keep(4, 0)));
    output.push("0123");
    output.push("4");

    assert_eq!(texts(&output.content), ["0123"]);
    assert_eq!(output.total_bytes(), 5);
}

#[test]
fn the_end_is_the_last_bytes_of_what_followed_the_start() {
    let output = fed(Some(keep(10, 5)), SIXTEEN);

    assert_eq!(texts(&output.content), ["0123456789"]);
    assert_eq!(output.tail, "bcdef");
    assert_eq!(output.kept_bytes(), 15);
    assert_eq!(output.total_bytes(), 16);
}

#[test]
fn the_end_holds_nothing_of_the_start() {
    let output = fed(Some(keep(10, 5)), "0123456789abc");

    assert_eq!(texts(&output.content), ["0123456789"]);
    assert_eq!(output.tail, "abc");
    assert_eq!(output.kept_bytes(), 13);
}

#[test]
fn the_end_is_one_run_of_text_whatever_parts_and_items_it_came_from() {
    let mut output = KeptOutput::new(Some(keep(4, 6)));
    output.push("012345");
    output.push("67");
    output.item();
    output.push("89");
    output.item();
    output.push("abcdef");

    assert_eq!(texts(&output.content), ["0123"]);
    assert_eq!(output.tail, "abcdef");
    assert_eq!(output.total_bytes(), 16);

    output.push("gh");
    assert_eq!(output.tail, "cdefgh");
}

#[test]
fn the_end_begins_at_the_first_character_boundary_its_bytes_allow() {
    let output = fed(Some(keep(0, 5)), EUROS);

    assert_eq!(output.tail, "\u{20ac}");
    assert_eq!(output.total_bytes(), 18);
}

/// The character that doesn't fit takes everything before it with it: the
/// end is the output's own ending with nothing missing.
#[test]
fn the_end_never_holds_text_from_before_a_character_it_could_not_hold() {
    let mut output = KeptOutput::new(Some(keep(0, 3)));
    output.push("xy");
    assert_eq!(output.tail, "xy");

    output.push("\u{20ac}a");

    assert_eq!(output.tail, "a");
}

#[test]
fn a_part_longer_than_the_end_is_held_from_its_own_last_bytes() {
    let mut output = KeptOutput::new(Some(keep(0, 4)));
    output.push("xy");
    output.push("0123456789");

    assert_eq!(output.tail, "6789");
}

// T14, the accumulator's half: an executor given a limit of 4 bytes at each
// end holds at most 8 bytes of a tool's 1 MiB and reports all of it.

const MEBIBYTE: usize = 1024 * 1024;

fn assert_holds_eight_bytes_of_a_mebibyte(output: &KeptOutput) {
    assert_eq!(output.total_bytes(), MEBIBYTE as u64);
    assert_eq!(output.kept_bytes(), 8);
    assert_eq!(texts(&output.content), ["0123"]);
    assert_eq!(output.tail, "cdef");
    let ToolResultContent::Text(start) = &output.content[0];
    assert!(
        start.capacity() < 64 && output.tail.capacity() < 64,
        "it holds no memory for the text it let go: {} and {} bytes",
        start.capacity(),
        output.tail.capacity()
    );
}

#[test]
fn a_mebibyte_written_at_once_is_counted_whole_and_held_at_its_limits() {
    let written = SIXTEEN.repeat(MEBIBYTE / 16);
    let mut output = KeptOutput::new(Some(keep(4, 4)));

    output.push(&written);

    assert_holds_eight_bytes_of_a_mebibyte(&output);
}

#[test]
fn a_mebibyte_written_in_parts_is_counted_whole_and_held_at_its_limits() {
    let part = SIXTEEN.repeat(1024 / 16);
    let mut output = KeptOutput::new(Some(keep(4, 4)));

    for _ in 0..1024 {
        output.push(&part);
    }

    assert_holds_eight_bytes_of_a_mebibyte(&output);
}

#[test]
fn a_result_the_loop_wrote_is_held_whole_as_one_item() {
    let output = KeptOutput::whole(SIXTEEN);

    assert_eq!(output, fed(None, SIXTEEN));
    assert_eq!(output.total_bytes(), 16);
    assert_eq!(output.kept_bytes(), 16);
    assert_eq!(KeptOutput::whole(""), KeptOutput::new(None));
}

// The cut.

const CUTS: [OutputCut; 3] = [
    OutputCut::Head,
    OutputCut::HeadTail,
    OutputCut::Preview { bytes: 4 },
];

#[test]
fn an_output_no_longer_than_the_cap_is_sent_whole_whatever_the_cut() {
    for cut in CUTS {
        for max_bytes in [16, 17, u64::MAX] {
            assert_eq!(
                sent(SIXTEEN, cap(max_bytes, cut)),
                (strings(&[SIXTEEN]), None),
                "{cut:?} at {max_bytes}"
            );
        }
    }
}

#[test]
fn an_output_one_byte_longer_than_the_cap_is_cut_whatever_the_cut() {
    for cut in CUTS {
        let (content, truncated_from_bytes) = sent(SIXTEEN, cap(15, cut));

        assert_eq!(truncated_from_bytes, Some(16), "{cut:?}");
        assert_ne!(content, [SIXTEEN], "{cut:?}");
    }
}

#[test]
fn without_a_cap_an_output_is_sent_whole() {
    let (content, truncated_from_bytes) = fed(None, SIXTEEN).cut(None);

    assert_eq!(texts(&content), [SIXTEEN]);
    assert_eq!(truncated_from_bytes, None);
}

#[test]
fn an_output_of_nothing_is_sent_as_nothing() {
    for cap in [None, Some(cap(0, OutputCut::HeadTail))] {
        assert_eq!(KeptOutput::new(None).cut(cap), (Vec::new(), None));
    }
}

#[test]
fn every_item_of_an_output_within_the_cap_is_sent_as_it_was_fed() {
    let output = fed_in_steps(Some(keep(16, 8)), &["aaaa", "bbbbbbbb", "cccc"], 3);

    let (content, truncated_from_bytes) = output.cut(Some(cap(16, OutputCut::HeadTail)));

    assert_eq!(texts(&content), ["aaaa", "bbbbbbbb", "cccc"]);
    assert_eq!(truncated_from_bytes, None);
}

#[test]
fn head_sends_the_start_up_to_the_cap_and_a_line_that_says_how_much_that_is() {
    assert_eq!(
        sent(SIXTEEN, cap(10, OutputCut::Head)),
        (
            strings(&["0123456789", "[truncated: the first 10 of 16 bytes]"]),
            Some(16)
        )
    );
}

#[test]
fn head_tail_sends_half_the_cap_from_each_end_around_a_line_that_says_what_was_left_out() {
    assert_eq!(
        sent(SIXTEEN, cap(10, OutputCut::HeadTail)),
        (
            strings(&["01234", "[truncated: 6 of 16 bytes left out]", "bcdef"]),
            Some(16)
        )
    );
}

#[test]
fn head_tail_rounds_half_an_odd_cap_down() {
    assert_eq!(
        sent(SIXTEEN, cap(11, OutputCut::HeadTail)),
        (
            strings(&["01234", "[truncated: 6 of 16 bytes left out]", "bcdef"]),
            Some(16)
        )
    );
}

/// Three bytes followed the ten the start kept, so the last five of the
/// output are those three and the two before them.
#[test]
fn the_end_takes_the_end_of_the_start_when_less_than_half_a_cap_followed_it() {
    assert_eq!(
        sent("0123456789abc", cap(10, OutputCut::HeadTail)),
        (
            strings(&["01234", "[truncated: 3 of 13 bytes left out]", "89abc"]),
            Some(13)
        )
    );
}

/// Six bytes followed the start and the end could hold one character of
/// them, so text is missing between the two and the end is that character
/// alone.
#[test]
fn the_end_takes_nothing_of_the_start_when_text_is_missing_between_them() {
    assert_eq!(
        sent("0123456789\u{20ac}\u{20ac}", cap(10, OutputCut::HeadTail)),
        (
            strings(&["01234", "[truncated: 8 of 16 bytes left out]", "\u{20ac}"]),
            Some(16)
        )
    );
}

#[test]
fn preview_sends_its_own_few_bytes_and_a_line_that_says_how_large_the_output_was() {
    assert_eq!(
        sent(SIXTEEN, cap(10, OutputCut::Preview { bytes: 4 })),
        (
            strings(&["0123", "[output too large: the first 4 of 16 bytes]"]),
            Some(16)
        )
    );
}

#[test]
fn the_lines_are_worded_as_the_spec_words_them() {
    assert_eq!(
        cap(50_000, OutputCut::Head).line(50_000, 0, 5_242_880),
        ToolResultContent::Text("[truncated: the first 50000 of 5242880 bytes]".to_owned())
    );
    assert_eq!(
        cap(50_000, OutputCut::HeadTail).line(25_000, 25_000, 5_242_880),
        ToolResultContent::Text("[truncated: 5192880 of 5242880 bytes left out]".to_owned())
    );
    assert_eq!(
        cap(50_000, OutputCut::Preview { bytes: 2_000 }).line(2_000, 0, 5_242_880),
        ToolResultContent::Text("[output too large: the first 2000 of 5242880 bytes]".to_owned())
    );
}

#[test]
fn no_cut_falls_inside_a_character_and_the_line_counts_what_was_sent() {
    assert_eq!(
        sent(EUROS, cap(10, OutputCut::Head)),
        (
            strings(&[
                "\u{20ac}\u{20ac}\u{20ac}",
                "[truncated: the first 9 of 18 bytes]"
            ]),
            Some(18)
        )
    );
    assert_eq!(
        sent(EUROS, cap(10, OutputCut::HeadTail)),
        (
            strings(&[
                "\u{20ac}",
                "[truncated: 12 of 18 bytes left out]",
                "\u{20ac}"
            ]),
            Some(18)
        )
    );
    assert_eq!(
        sent(EUROS, cap(10, OutputCut::Preview { bytes: 4 })),
        (
            strings(&["\u{20ac}", "[output too large: the first 3 of 18 bytes]"]),
            Some(18)
        )
    );
}

#[test]
fn the_cap_is_on_the_text_as_a_whole_its_items_in_order() {
    let items = ["aaaa", "bbbbbbbb", "cccc"];
    let cut = |cap: OutputCap| {
        let (content, truncated_from_bytes) =
            fed_in_steps(Some(cap.keeps()), &items, 3).cut(Some(cap));
        assert_eq!(truncated_from_bytes, Some(16));
        texts(&content)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    assert_eq!(
        cut(cap(10, OutputCut::Head)),
        ["aaaa", "bbbbbb", "[truncated: the first 10 of 16 bytes]"]
    );
    assert_eq!(
        cut(cap(10, OutputCut::HeadTail)),
        ["aaaa", "b", "[truncated: 6 of 16 bytes left out]", "bcccc"]
    );
    assert_eq!(
        cut(cap(10, OutputCut::Preview { bytes: 4 })),
        ["aaaa", "[output too large: the first 4 of 16 bytes]"]
    );
}

/// The second item would fit in the two bytes the first left over, and
/// sending it would put it straight after text it didn't follow.
#[test]
fn an_item_after_the_one_that_was_cut_is_not_sent() {
    let output = fed_in_steps(None, &["\u{20ac}\u{20ac}", "ab"], 1);

    let (content, truncated_from_bytes) = output.cut(Some(cap(5, OutputCut::Head)));

    assert_eq!(
        texts(&content),
        ["\u{20ac}", "[truncated: the first 3 of 8 bytes]"]
    );
    assert_eq!(truncated_from_bytes, Some(8));
}

#[test]
fn a_cap_too_small_to_send_anything_leaves_the_line() {
    assert_eq!(
        sent("0123456789", cap(0, OutputCut::Head)),
        (strings(&["[truncated: the first 0 of 10 bytes]"]), Some(10))
    );
    assert_eq!(
        sent("0123456789", cap(1, OutputCut::HeadTail)),
        (strings(&["[truncated: 10 of 10 bytes left out]"]), Some(10))
    );
    assert_eq!(
        sent("0123456789", cap(0, OutputCut::Preview { bytes: 0 })),
        (
            strings(&["[output too large: the first 0 of 10 bytes]"]),
            Some(10)
        )
    );
}

// An executor that kept less than the run's cap asks for. The port forbids
// it and no type can, so the cut has to be honest about what it was given.

#[test]
fn an_output_that_was_not_all_kept_says_so_in_a_run_without_a_cap() {
    let output = fed(Some(keep(4, 0)), "0123456789");

    assert_eq!(
        output.cut(None),
        (
            vec![
                ToolResultContent::Text("0123".to_owned()),
                ToolResultContent::Text("[truncated: the first 4 of 10 bytes]".to_owned()),
            ],
            Some(10)
        )
    );
}

#[test]
fn an_output_within_the_cap_that_was_not_all_kept_is_cut_from_what_was_kept() {
    let kept = || fed(Some(keep(4, 3)), "0123456789");

    let (head, _) = kept().cut(Some(cap(100, OutputCut::Head)));
    let (head_tail, truncated_from_bytes) = kept().cut(Some(cap(100, OutputCut::HeadTail)));
    let (preview, _) = kept().cut(Some(cap(100, OutputCut::Preview { bytes: 2 })));

    assert_eq!(
        texts(&head),
        ["0123", "[truncated: the first 4 of 10 bytes]"]
    );
    assert_eq!(
        texts(&head_tail),
        ["0123", "[truncated: 3 of 10 bytes left out]", "789"]
    );
    assert_eq!(truncated_from_bytes, Some(10));
    assert_eq!(
        texts(&preview),
        ["01", "[output too large: the first 2 of 10 bytes]"]
    );
}

#[test]
fn the_end_that_is_sent_is_no_longer_than_the_cut_allows() {
    let output = fed(Some(keep(4, 6)), SIXTEEN);

    let (content, _) = output.cut(Some(cap(8, OutputCut::HeadTail)));

    assert_eq!(
        texts(&content),
        ["0123", "[truncated: 8 of 16 bytes left out]", "cdef"]
    );
}

#[test]
fn an_output_kept_in_two_parts_with_nothing_missing_is_sent_whole() {
    for cap in [None, Some(cap(6, OutputCut::HeadTail))] {
        assert_eq!(
            fed(Some(keep(4, 4)), "012345").cut(cap),
            (vec![ToolResultContent::Text("012345".to_owned())], None),
            "{cap:?}"
        );
    }
    assert_eq!(
        fed(Some(keep(0, 4)), "ab").cut(None),
        (vec![ToolResultContent::Text("ab".to_owned())], None)
    );
}

// The closing line.

const CLOSING: &str = "exit code: 3";

/// `text` fed a character at a time, and `closing` said of it.
fn closed(keep: Option<OutputKeep>, text: &str, closing: &str) -> KeptOutput {
    let mut output = fed(keep, text);
    output.close(closing);
    output
}

/// What the model is sent of `text` and its closing line when an executor
/// kept what `cap` asks for, and the size the outcome holds.
fn sent_closed(text: &str, cap: OutputCap) -> (Vec<String>, Option<u64>) {
    let (content, truncated_from_bytes) = closed(Some(cap.keeps()), text, CLOSING).cut(Some(cap));
    let content = texts(&content).into_iter().map(str::to_owned).collect();
    (content, truncated_from_bytes)
}

#[test]
fn a_closing_line_is_held_apart_from_the_text_and_counts_as_fed_and_as_kept() {
    let output = closed(Some(keep(10, 5)), SIXTEEN, CLOSING);

    assert_eq!(texts(&output.content), ["0123456789"]);
    assert_eq!(output.tail, "bcdef");
    assert_eq!(output.closing, CLOSING);
    assert_eq!(output.total_bytes(), 16 + 12);
    assert_eq!(output.kept_bytes(), 10 + 5 + 12);
}

#[test]
fn every_cut_sends_the_closing_line_after_everything_else() {
    assert_eq!(
        sent_closed(SIXTEEN, cap(10, OutputCut::Head)),
        (
            strings(&[
                "0123456789",
                "[truncated: the first 10 of 28 bytes]",
                CLOSING
            ]),
            Some(28)
        )
    );
    assert_eq!(
        sent_closed(SIXTEEN, cap(10, OutputCut::HeadTail)),
        (
            strings(&[
                "01234",
                "[truncated: 6 of 28 bytes left out]",
                "bcdef",
                CLOSING
            ]),
            Some(28)
        )
    );
    assert_eq!(
        sent_closed(SIXTEEN, cap(10, OutputCut::Preview { bytes: 4 })),
        (
            strings(&[
                "0123",
                "[output too large: the first 4 of 28 bytes]",
                CLOSING
            ]),
            Some(28)
        )
    );
}

#[test]
fn an_output_within_the_cap_is_sent_as_if_its_closing_line_had_been_fed_last() {
    for cut in CUTS {
        for max_bytes in [28, 29, u64::MAX] {
            let cap = cap(max_bytes, cut);
            let closed = closed(Some(cap.keeps()), SIXTEEN, CLOSING);
            let mut pushed = fed(Some(cap.keeps()), SIXTEEN);
            pushed.push(CLOSING);

            assert_eq!(closed.total_bytes(), pushed.total_bytes(), "{cap:?}");
            assert_eq!(closed.kept_bytes(), pushed.kept_bytes(), "{cap:?}");
            let sent = closed.cut(Some(cap));
            assert_eq!(sent, pushed.cut(Some(cap)), "{cap:?}");
            assert_eq!(
                sent,
                (
                    vec![ToolResultContent::Text(
                        "0123456789abcdefexit code: 3".to_owned()
                    )],
                    None
                ),
                "{cap:?}"
            );
        }
    }
}

/// The closing line is sent on top of the cap, as the line that says what
/// was left out is, so an output that lost nothing doesn't say it did.
#[test]
fn an_output_whose_text_fits_the_cap_is_sent_whole_whatever_its_closing_line_adds() {
    for cut in CUTS {
        for max_bytes in [16, 27] {
            assert_eq!(
                sent_closed(SIXTEEN, cap(max_bytes, cut)),
                (strings(&["0123456789abcdefexit code: 3"]), None),
                "{cut:?} at {max_bytes}"
            );
        }
    }
}

#[test]
fn an_output_whose_text_is_one_byte_longer_than_the_cap_is_cut_and_closed() {
    assert_eq!(
        sent_closed(SIXTEEN, cap(15, OutputCut::Head)),
        (
            strings(&[
                "0123456789abcde",
                "[truncated: the first 15 of 28 bytes]",
                CLOSING
            ]),
            Some(28)
        )
    );
}

#[test]
fn a_closing_line_goes_on_from_the_end_of_an_output_kept_in_two_parts() {
    let output = closed(Some(keep(4, 4)), "012345", CLOSING);

    assert_eq!(
        output.cut(None),
        (
            vec![ToolResultContent::Text("012345exit code: 3".to_owned())],
            None
        )
    );
}

/// The start kept nothing of the item, so nothing told it that the item
/// has text, which the line goes on from as it would have had it been fed.
#[test]
fn a_closing_line_goes_on_from_an_item_that_is_all_held_at_the_end() {
    let closed = closed(Some(keep(0, 16)), "ab", CLOSING);
    let mut pushed = fed(Some(keep(0, 16)), "ab");
    pushed.push(CLOSING);

    let sent = closed.cut(None);

    assert_eq!(
        sent,
        (
            vec![ToolResultContent::Text("abexit code: 3".to_owned())],
            None
        )
    );
    assert_eq!(sent, pushed.cut(None));
}

#[test]
fn a_closing_line_is_an_item_of_its_own_where_the_text_before_it_ended_an_item() {
    let mut after_an_item = KeptOutput::whole("abcd");
    after_an_item.item();
    after_an_item.close(CLOSING);
    let mut of_nothing = KeptOutput::new(None);
    of_nothing.close(CLOSING);

    let (after_an_item, truncated_from_bytes) = after_an_item.cut(None);
    let (of_nothing, _) = of_nothing.cut(Some(cap(12, OutputCut::Head)));

    assert_eq!(texts(&after_an_item), ["abcd", CLOSING]);
    assert_eq!(truncated_from_bytes, None);
    assert_eq!(texts(&of_nothing), [CLOSING]);
}

#[test]
fn a_closing_line_replaces_the_one_said_before() {
    let mut output = KeptOutput::whole("abcd\n");
    output.close("exit code: 0");
    output.close("signal: 9");

    assert_eq!(output.total_bytes(), 5 + 9);
    assert_eq!(output.kept_bytes(), 5 + 9);
    assert_eq!(
        output.clone().cut(None),
        (
            vec![ToolResultContent::Text("abcd\nsignal: 9".to_owned())],
            None
        )
    );

    output.close("");

    assert_eq!(output, KeptOutput::whole("abcd\n"));
}

#[test]
fn no_more_of_a_closing_line_is_kept_than_its_limit_up_to_a_character_boundary() {
    assert_eq!(KeptOutput::CLOSING_MAX_BYTES, 256);
    let kept_of = |line: &str| {
        let mut output = KeptOutput::new(None);
        output.close(line);
        assert_eq!(output.total_bytes(), output.closing.len() as u64);
        assert_eq!(output.kept_bytes(), output.closing.len() as u64);
        output.closing
    };
    let at_the_limit = format!("{}\u{20ac}", "x".repeat(253));

    assert_eq!(kept_of(&at_the_limit), at_the_limit);
    assert_eq!(kept_of(&"x".repeat(300)), "x".repeat(256));
    assert_eq!(
        kept_of(&format!("{}{EUROS}", "x".repeat(254))),
        "x".repeat(254),
        "the limit falls inside the first of the characters"
    );
}

/// The line is sent on top of the cap, which is why it has a limit of its
/// own.
#[test]
fn a_closing_line_is_sent_whole_under_a_cap_that_sends_nothing_else() {
    assert_eq!(
        sent_closed(SIXTEEN, cap(0, OutputCut::Head)),
        (
            strings(&["[truncated: the first 0 of 28 bytes]", CLOSING]),
            Some(28)
        )
    );
}

#[test]
fn an_output_that_was_not_all_kept_is_cut_and_still_closed() {
    let kept = || closed(Some(keep(4, 3)), "0123456789", CLOSING);

    let (uncapped, truncated_from_bytes) = kept().cut(None);
    let (head_tail, _) = kept().cut(Some(cap(100, OutputCut::HeadTail)));

    assert_eq!(
        texts(&uncapped),
        ["0123", "[truncated: the first 4 of 22 bytes]", CLOSING]
    );
    assert_eq!(truncated_from_bytes, Some(22));
    assert_eq!(
        texts(&head_tail),
        [
            "0123",
            "[truncated: 3 of 22 bytes left out]",
            "789",
            CLOSING
        ]
    );
}

fn any_cap() -> impl proptest::strategy::Strategy<Value = OutputCap> {
    use proptest::strategy::Strategy;

    (0_u64..40).prop_flat_map(|max_bytes| {
        proptest::prop_oneof![
            proptest::strategy::Just(OutputCut::Head),
            proptest::strategy::Just(OutputCut::HeadTail),
            (0..=max_bytes).prop_map(|bytes| OutputCut::Preview { bytes }),
        ]
        .prop_map(move |cut| cap(max_bytes, cut))
    })
}

/// Items of one to four bytes a character, so every limit lands inside a
/// character somewhere.
fn any_items() -> impl proptest::strategy::Strategy<Value = Vec<String>> {
    proptest::collection::vec("[ab\u{e9}\u{20ac}\u{1d11e}]{1,16}", 0..4)
}

/// A closing line, which half of the outputs have none of.
fn any_closing() -> impl proptest::strategy::Strategy<Value = String> {
    proptest::prop_oneof![
        proptest::strategy::Just(String::new()),
        "[ab\u{e9}\u{20ac}\u{1d11e}]{1,8}",
    ]
}

/// `items` fed as [`fed_in_steps`] feeds them, and `closing` said of them.
fn closed_in_steps(
    keep: Option<OutputKeep>,
    items: &[&str],
    step: usize,
    closing: &str,
) -> KeptOutput {
    let mut output = fed_in_steps(keep, items, step);
    output.close(closing);
    output
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// What an executor keeps is enough: the model is sent the same from
    /// what was kept as it would be from the whole output, however the text
    /// arrived, and the executor never held more than its limits.
    #[test]
    fn what_was_kept_is_cut_as_the_whole_output_would_be(
        items in any_items(), closing in any_closing(), cap in any_cap(), step in 1_usize..6
    ) {
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        let keeps = cap.keeps();

        let kept = closed_in_steps(Some(keeps), &items, step, &closing);
        let whole = closed_in_steps(None, &items, 16, &closing);

        proptest::prop_assert!(
            kept.kept_bytes() <= keeps.head + keeps.tail + closing.len() as u64
        );
        proptest::prop_assert_eq!(kept.total_bytes(), whole.total_bytes());
        proptest::prop_assert_eq!(kept.cut(Some(cap)), whole.cut(Some(cap)));
    }

    /// An output is cut exactly when its text is longer than the cap, which
    /// the closing line isn't counted against. What's sent of one that's cut
    /// is the output's own start and its own end, within the cap between
    /// them, the line, and the closing line whole. The start and the end are
    /// each as long as the cut allows, in whole characters, so a cut that
    /// sends less than the cap allows fails as one that sends more does.
    #[test]
    fn what_is_sent_is_the_start_and_the_end_of_the_output_within_the_cap(
        items in any_items(), closing in any_closing(), cap in any_cap(), step in 1_usize..6
    ) {
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        let text = items.concat();
        let total = (text.len() + closing.len()) as u64;

        let (content, truncated_from_bytes) =
            closed_in_steps(Some(cap.keeps()), &items, step, &closing).cut(Some(cap));

        proptest::prop_assert_eq!(
            truncated_from_bytes,
            (text.len() as u64 > cap.max_bytes).then_some(total)
        );
        let sent = texts(&content);
        if truncated_from_bytes.is_none() {
            let mut fed_last = fed_in_steps(None, &items, 16);
            fed_last.push(&closing);
            proptest::prop_assert_eq!(sent, texts(&fed_last.content));
        } else {
            let line = sent.iter().position(|text| text.starts_with('[')).unwrap();
            let after = &sent[line + 1..];
            let after = if closing.is_empty() {
                after
            } else {
                proptest::prop_assert_eq!(after.last(), Some(&closing.as_str()));
                &after[..after.len() - 1]
            };
            let (start, end) = (sent[..line].concat(), after.concat());
            proptest::prop_assert!(text.starts_with(&start), "{start:?} of {text:?}");
            proptest::prop_assert!(text.ends_with(&end), "{end:?} of {text:?}");
            proptest::prop_assert!(after.len() <= 1);
            proptest::prop_assert!((start.len() + end.len()) as u64 <= cap.max_bytes);
            // What spec section 1 says each cut sends, and not what the cap
            // says it sends, which is the code under test.
            let (head, tail) = match cap.cut {
                OutputCut::Head => (cap.max_bytes, 0),
                OutputCut::HeadTail => (cap.max_bytes / 2, cap.max_bytes / 2),
                OutputCut::Preview { bytes } => (bytes, 0),
            };
            let (head, tail) = (usize::try_from(head).unwrap(), usize::try_from(tail).unwrap());
            proptest::prop_assert_eq!(start.len(), text.floor_char_boundary(head));
            proptest::prop_assert_eq!(
                end.len(),
                text.len() - text.ceil_char_boundary(text.len().saturating_sub(tail))
            );
            let ToolResultContent::Text(worded) =
                cap.line(start.len() as u64, (end.len() + closing.len()) as u64, total);
            proptest::prop_assert_eq!(sent[line], worded);
        }
    }
}
