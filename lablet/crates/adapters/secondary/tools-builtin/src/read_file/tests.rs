use super::*;

const FILE: &str = "one\ntwo\nthree\nfour\nfive";

/// What `lines` takes of the file, read in pieces of `size` bytes, and how
/// many pieces were read before every line asked for had been.
fn taken(mut lines: Lines, size: usize) -> (String, usize) {
    let mut taken = Vec::new();
    let mut pieces = 0;
    for piece in FILE.as_bytes().chunks(size) {
        if lines.are_read() {
            break;
        }
        pieces += 1;
        taken.extend_from_slice(lines.of(piece));
    }
    (String::from_utf8(taken).unwrap(), pieces)
}

#[test]
fn the_lines_asked_for_are_taken_however_the_file_is_cut_into_pieces() {
    for (skip, take, expected) in [
        (0, None, FILE),
        (0, Some(2), "one\ntwo\n"),
        (1, Some(2), "two\nthree\n"),
        (3, None, "four\nfive"),
        (4, Some(1), "five"),
        (4, Some(9), "five"),
        (5, None, ""),
        (9, Some(1), ""),
    ] {
        for size in 1..=FILE.len() {
            let (taken, _) = taken(Lines { skip, take }, size);
            assert_eq!(
                taken, expected,
                "skipping {skip} and taking {take:?}, in pieces of {size}"
            );
        }
    }
}

#[test]
fn nothing_is_read_after_the_last_line_asked_for() {
    let lines = Lines {
        skip: 0,
        take: Some(1),
    };

    let (taken, pieces) = taken(lines, 4);

    assert_eq!(taken, "one\n");
    assert_eq!(pieces, 1, "the first piece holds the whole line");
}
