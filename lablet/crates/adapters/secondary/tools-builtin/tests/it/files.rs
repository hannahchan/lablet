use std::time::{Duration, Instant};

use lablet_model::OutputKeep;
use lablet_run::{ToolErrorKind, ToolExecutor};
use lablet_tools_builtin::BuiltinTools;
use serde_json::{Value, json};

use crate::harness::{Scratch, TIMEOUT, call, keeping, link, refused, said, within};

const FILE: &str = "one\ntwo\nthree\nfour\nfive";

/// What a file outside the root holds, which no result may.
const SECRET: &str = "what the model is not to read";

/// A root beside a directory that holds a secret, and the tools under the
/// root.
fn beside_a_secret(test: &str) -> (Scratch, BuiltinTools) {
    let scratch = Scratch::new(test);
    std::fs::create_dir(scratch.outside("private")).unwrap();
    std::fs::write(scratch.outside("private/secret.txt"), SECRET).unwrap();
    let tools = scratch.tools();
    (scratch, tools)
}

fn secret(scratch: &Scratch) -> String {
    std::fs::read_to_string(scratch.outside("private/secret.txt")).unwrap()
}

#[tokio::test]
async fn a_file_is_read_whole_by_a_path_from_the_root_or_an_absolute_one() {
    let scratch = Scratch::new("read-whole");
    let tools = scratch.tools();
    let absolute = scratch.holds("notes/plan.txt", FILE);

    let from_the_root = said(&tools, "read_file", json!({ "path": "notes/plan.txt" })).await;
    let by_the_absolute = said(&tools, "read_file", json!({ "path": absolute })).await;
    let through_the_parent = said(
        &tools,
        "read_file",
        json!({ "path": "notes/../notes/./plan.txt" }),
    )
    .await;

    assert_eq!(from_the_root, FILE);
    assert_eq!(by_the_absolute, FILE);
    assert_eq!(through_the_parent, FILE);
}

#[tokio::test]
async fn the_lines_after_the_offset_are_read_up_to_the_limit() {
    let scratch = Scratch::new("read-lines");
    let tools = scratch.tools();
    scratch.holds("plan.txt", FILE);

    for (lines, expected) in [
        (json!({ "limit": 2 }), "one\ntwo\n"),
        (json!({ "offset": 1, "limit": 2 }), "two\nthree\n"),
        (json!({ "offset": 3 }), "four\nfive"),
        (json!({ "offset": 0, "limit": 99 }), FILE),
        (json!({ "offset": 5, "limit": 1 }), ""),
    ] {
        let mut input = json!({ "path": "plan.txt" });
        input
            .as_object_mut()
            .unwrap()
            .extend(lines.as_object().unwrap().clone());

        let text = said(&tools, "read_file", input).await;

        assert_eq!(text, expected, "{lines}");
    }
}

#[tokio::test]
async fn a_long_file_is_read_in_parts_that_make_the_whole() {
    let scratch = Scratch::new("read-parts");
    let tools = scratch.tools();
    let file = (0..40_000).fold(String::new(), |file, line| file + &format!("line {line}\n"));
    scratch.holds("long.txt", &file);

    let mut parts = String::new();
    for part in 0..4 {
        parts += &said(
            &tools,
            "read_file",
            json!({ "path": "long.txt", "offset": part * 10_000, "limit": 10_000 }),
        )
        .await;
    }

    assert!(parts == file, "the parts aren't the file");
}

#[tokio::test]
async fn bytes_of_a_file_that_are_no_text_are_returned_as_the_character_that_says_so() {
    let scratch = Scratch::new("read-bytes");
    let tools = scratch.tools();
    scratch.holds("data.bin", b"a\xFFb\n\xE2\x82");

    let text = said(&tools, "read_file", json!({ "path": "data.bin" })).await;

    assert_eq!(text, "a\u{FFFD}b\n\u{FFFD}");
}

#[tokio::test]
async fn a_read_that_takes_longer_than_the_call_may_is_given_up() {
    let scratch = Scratch::new("read-deadline");
    let tools = scratch.tools();
    // A file of nothing but a size, which takes no room and is too long to
    // be read in the millisecond the call has.
    let long = std::fs::File::create(scratch.root().join("long.bin")).unwrap();
    long.set_len(1 << 30).unwrap();
    let deadline = Duration::from_millis(1);
    let keep = OutputKeep { head: 4, tail: 4 };

    let began = Instant::now();
    let error = tools
        .execute(within(
            deadline,
            keeping(keep, call("read_file", json!({ "path": "long.bin" }))),
        ))
        .await
        .unwrap_err();
    let took = began.elapsed();

    assert_eq!(error.kind, ToolErrorKind::Timeout);
    assert_eq!(
        error.message(),
        "read_file was stopped after 1ms, the longest the call could take"
    );
    assert!(deadline <= took && took < TIMEOUT, "it took {took:?}");
}

#[tokio::test]
async fn a_path_that_leads_outside_the_root_is_refused_and_the_file_is_not_read() {
    let (scratch, tools) = beside_a_secret("read-outside");
    std::fs::create_dir(scratch.root().join("sub")).unwrap();
    let absolute = scratch.outside("private/secret.txt");

    for path in [
        "../private/secret.txt",
        "sub/../../private/secret.txt",
        absolute.to_str().unwrap(),
    ] {
        let text = refused(&tools, "read_file", json!({ "path": path })).await;

        assert_eq!(
            text,
            format!("{path} wasn't read: it leads outside the run's root directory")
        );
    }
    assert_eq!(secret(&scratch), SECRET, "the file is there to be read");
}

#[tokio::test]
async fn a_link_under_the_root_that_leads_outside_it_is_refused_and_the_file_is_not_read() {
    let (scratch, tools) = beside_a_secret("read-link");
    link(
        &scratch.outside("private/secret.txt"),
        &scratch.root().join("notes.txt"),
    );
    link(&scratch.outside("private"), &scratch.root().join("linked"));
    assert_eq!(
        std::fs::read_to_string(scratch.root().join("notes.txt")).unwrap(),
        SECRET,
        "the link leads to the file"
    );

    for path in ["notes.txt", "linked/secret.txt"] {
        let text = refused(&tools, "read_file", json!({ "path": path })).await;

        assert_eq!(
            text,
            format!("{path} wasn't read: it leads outside the run's root directory")
        );
    }
}

#[tokio::test]
async fn a_link_that_stays_under_the_root_is_followed() {
    let scratch = Scratch::new("read-link-inside");
    let tools = scratch.tools();
    let file = scratch.holds("real/plan.txt", FILE);
    link(&file, &scratch.root().join("plan.txt"));
    link(&scratch.root().join("real"), &scratch.root().join("linked"));

    let by_a_link = said(&tools, "read_file", json!({ "path": "plan.txt" })).await;
    let through_a_link = said(&tools, "read_file", json!({ "path": "linked/plan.txt" })).await;

    assert_eq!(by_a_link, FILE);
    assert_eq!(through_a_link, FILE);
}

#[tokio::test]
async fn what_cannot_be_read_is_an_error_result_that_says_why() {
    let scratch = Scratch::new("read-refused");
    let tools = scratch.tools();
    scratch.holds("sub/plan.txt", FILE);

    for (path, why) in [
        ("missing.txt", "no such file or directory"),
        ("sub", "it isn't a file"),
        ("", "it isn't a file"),
        ("sub/plan.txt/more", "Not a directory (os error 20)"),
    ] {
        let text = refused(&tools, "read_file", json!({ "path": path })).await;

        assert_eq!(text, format!("{path} wasn't read: {why}"));
    }
}

#[tokio::test]
async fn arguments_of_a_read_that_do_not_fit_are_an_error_result_that_says_what_is_wrong() {
    let scratch = Scratch::new("read-arguments");
    let tools = scratch.tools();
    scratch.holds("plan.txt", FILE);

    for (input, says) in [
        (json!({}), "missing field `path`"),
        (json!({ "path": "plan.txt", "limit": 0 }), "nonzero"),
        (json!({ "path": "plan.txt", "offset": -1 }), "invalid value"),
        (json!({ "path": "plan.txt", "lines": 2 }), "unknown field"),
    ] {
        let text = refused(&tools, "read_file", input).await;

        assert!(
            text.starts_with("the arguments of read_file don't fit: ") && text.contains(says),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_file_is_written_with_the_directories_on_the_way_to_it() {
    let scratch = Scratch::new("write-new");
    let tools = scratch.tools();
    let absolute = scratch.root().join("by/absolute.txt");

    let said_of_the_first = said(
        &tools,
        "write_file",
        json!({ "path": "notes/2026/plan.txt", "content": FILE }),
    )
    .await;
    said(
        &tools,
        "write_file",
        json!({ "path": absolute, "content": "two" }),
    )
    .await;

    assert_eq!(said_of_the_first, "wrote 23 bytes to notes/2026/plan.txt");
    assert_eq!(scratch.read("notes/2026/plan.txt"), FILE);
    assert_eq!(scratch.read("by/absolute.txt"), "two");
}

#[tokio::test]
async fn a_file_that_is_there_is_replaced_by_what_is_written() {
    let scratch = Scratch::new("write-replace");
    let tools = scratch.tools();
    scratch.holds("plan.txt", FILE);

    said(
        &tools,
        "write_file",
        json!({ "path": "plan.txt", "content": "new" }),
    )
    .await;

    assert_eq!(scratch.read("plan.txt"), "new");
}

#[tokio::test]
async fn a_write_that_leads_outside_the_root_is_refused_and_nothing_is_written() {
    let (scratch, tools) = beside_a_secret("write-outside");
    let root = scratch.root();
    std::fs::create_dir(root.join("sub")).unwrap();
    link(&scratch.outside("private"), &root.join("linked"));
    link(
        &scratch.outside("private/secret.txt"),
        &root.join("notes.txt"),
    );
    link(&scratch.outside("private/made.txt"), &root.join("dangling"));
    link(&scratch.outside("private/no/such"), &root.join("nowhere"));
    let absolute = scratch.outside("private/made.txt");
    let outside = "it leads outside the run's root directory";
    let dangling = "it's a symbolic link to something that doesn't exist";

    for (path, why) in [
        ("../private/made.txt", outside),
        ("sub/../../private/made.txt", outside),
        (absolute.to_str().unwrap(), outside),
        ("../made/on/the/way.txt", outside),
        ("linked/made.txt", outside),
        ("linked/made/on/the/way.txt", outside),
        ("notes.txt", outside),
        ("dangling", dangling),
        ("nowhere/made.txt", dangling),
        (
            "missing/../../private/made.txt",
            "it goes through a directory that doesn't exist",
        ),
    ] {
        let text = refused(
            &tools,
            "write_file",
            json!({ "path": path, "content": "escaped" }),
        )
        .await;

        assert_eq!(text, format!("{path} wasn't written: {why}"));
    }
    assert_eq!(secret(&scratch), SECRET);
    let beside: Vec<_> = std::fs::read_dir(scratch.outside(""))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(beside.len(), 2, "beside the root: {beside:?}");
    let private: Vec<_> = std::fs::read_dir(scratch.outside("private"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(private, ["secret.txt"]);
    assert!(!root.join("missing").exists());
}

#[tokio::test]
async fn a_write_through_a_link_that_stays_under_the_root_is_followed() {
    let scratch = Scratch::new("write-link-inside");
    let tools = scratch.tools();
    scratch.holds("real/plan.txt", FILE);
    link(&scratch.root().join("real"), &scratch.root().join("linked"));
    link(
        &scratch.root().join("real/plan.txt"),
        &scratch.root().join("plan.txt"),
    );

    for (path, content) in [("linked/new/notes.txt", "notes"), ("plan.txt", "new")] {
        said(
            &tools,
            "write_file",
            json!({ "path": path, "content": content }),
        )
        .await;
    }

    assert_eq!(scratch.read("real/new/notes.txt"), "notes");
    assert_eq!(scratch.read("real/plan.txt"), "new");
    assert!(
        scratch
            .root()
            .join("plan.txt")
            .symlink_metadata()
            .unwrap()
            .is_symlink(),
        "the link is a link still"
    );
}

#[tokio::test]
async fn what_cannot_be_written_is_an_error_result_that_says_why() {
    let scratch = Scratch::new("write-refused");
    let tools = scratch.tools();
    scratch.holds("sub/plan.txt", FILE);

    for (path, why) in [
        ("sub", "it isn't a file"),
        ("", "it isn't a file"),
        ("sub/plan.txt/more.txt", "Not a directory (os error 20)"),
    ] {
        let text = refused(
            &tools,
            "write_file",
            json!({ "path": path, "content": "new" }),
        )
        .await;

        assert_eq!(text, format!("{path} wasn't written: {why}"));
    }
    assert_eq!(scratch.read("sub/plan.txt"), FILE);
}

#[tokio::test]
async fn arguments_of_a_write_that_do_not_fit_are_an_error_result_and_nothing_is_written() {
    let scratch = Scratch::new("write-arguments");
    let tools = scratch.tools();

    for (input, says) in [
        (json!({ "path": "plan.txt" }), "missing field `content`"),
        (json!({ "content": "new" }), "missing field `path`"),
        (
            json!({ "path": "plan.txt", "content": "new", "append": true }),
            "unknown field `append`",
        ),
        (Value::Null, "invalid type: null"),
    ] {
        let text = refused(&tools, "write_file", input).await;

        assert!(
            text.starts_with("the arguments of write_file don't fit: ") && text.contains(says),
            "{text}"
        );
    }
    assert!(!scratch.root().join("plan.txt").exists());
}
