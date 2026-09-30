use std::ffi::OsStr;
use std::time::Duration;

use lablet_model::{
    CacheScope, CompletionMode, ModelRef, Prompts, ProviderApi, RequestParams, Run, RunContext,
    RunLabels, RunSetup, StopReason, Thinking,
};

use super::*;

const FIRST: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
const SECOND: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";

fn id(run: &str) -> RunId {
    RunId::new(run).unwrap()
}

fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Script,
        name: "scripted".to_owned(),
        replays_reasoning: false,
    }
}

/// The document of a run that stopped before its first response, which is
/// as much of a run as a writer needs: its id, and a prompt to tell one
/// document from another by.
fn document(run: &str, system: &str) -> TranscriptDocument {
    let setup = RunSetup {
        run_id: id(run),
        labels: RunLabels::default(),
        model: model(),
        endpoint: None,
        tools: Vec::new(),
        tools_bytes: 0,
        tools_digest: String::new(),
        system_prompt_digest: String::new(),
        completion: CompletionMode::Natural,
        max_turns: None,
        timeout: Duration::from_secs(600),
        request: RequestParams {
            max_tokens: 4_096,
            temperature: None,
            thinking: Thinking::default(),
            effort: None,
            seed: None,
            cache_scope: CacheScope::default(),
        },
    };
    let context = RunContext {
        run_id: id(run),
        labels: RunLabels::default(),
        started_unix_ms: 1_790_000_000_123,
        config_digest: "9f2c".to_owned(),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    };
    let finished = Run::start(
        setup,
        Prompts::new(system, "Fix the failing test.").unwrap(),
    )
    .finish(
        StopReason::Cancelled,
        Duration::ZERO,
        None,
        None,
        None,
        None,
    );
    TranscriptDocument::new(context, model(), Vec::new(), finished.transcript)
}

fn compact(document: &TranscriptDocument) -> String {
    let mut written = serde_json::to_string(document).unwrap();
    written.push('\n');
    written
}

/// A directory of this test's own, removed when the test ends, so tests
/// that run together never write to one file.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "lablet-transcript-json-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Where a run's transcript goes

#[test]
fn the_run_id_takes_the_place_of_the_placeholder_wherever_the_path_holds_it() {
    for (configured, path) in [
        (
            "out/transcript-{run_id}.json",
            format!("out/transcript-{FIRST}.json"),
        ),
        ("{run_id}", FIRST.to_owned()),
        (
            "runs/{run_id}/{run_id}.json",
            format!("runs/{FIRST}/{FIRST}.json"),
        ),
        ("{run_id}{run_id}", format!("{FIRST}{FIRST}")),
    ] {
        let file = TranscriptFile::for_run(Path::new(configured), &id(FIRST)).unwrap();

        assert_eq!(file.path(), Path::new(&path), "{configured}");
    }
}

#[test]
fn a_path_that_holds_no_placeholder_is_left_as_it_is_and_is_every_run_s_file() {
    for configured in [
        "out/transcript.json",
        "out/{run_id.json",
        "out/{RUN_ID}.json",
        "out/{run-id}.json",
        "",
    ] {
        let first = TranscriptFile::for_run(Path::new(configured), &id(FIRST)).unwrap();
        let second = TranscriptFile::for_run(Path::new(configured), &id(SECOND)).unwrap();

        assert_eq!(first.path(), Path::new(configured));
        assert_eq!(first, second);
    }
}

/// The run id is put in and never read again, so an id that spells the
/// placeholder is a file name like any other.
#[test]
fn a_run_id_that_spells_the_placeholder_is_put_in_once() {
    let file = TranscriptFile::for_run(Path::new("out/{run_id}.json"), &id("{run_id}")).unwrap();

    assert_eq!(file.path(), Path::new("out/{run_id}.json"));
}

/// A caller names a run id, and the config names where transcripts go. An
/// id that isn't one component of a path would write somewhere else.
#[test]
fn a_run_id_that_is_not_one_path_component_is_refused_where_the_path_holds_the_placeholder() {
    for (run_id, reason) in [
        ("../../x", "it holds a `/`"),
        ("a/b", "it holds a `/`"),
        ("/etc/passwd", "it holds a `/`"),
        ("..", "it names the directory above"),
        (".", "it names the directory itself"),
        ("a\0b", "it holds a NUL"),
    ] {
        for configured in [
            "out/transcript-{run_id}.json",
            "out/{run_id}/transcript.json",
            "{run_id}",
        ] {
            let refused = TranscriptFile::for_run(Path::new(configured), &id(run_id));

            assert_eq!(
                refused,
                Err(TranscriptWriteError::RunIdNotOneComponent {
                    run_id: run_id.to_owned(),
                    reason,
                }),
                "{run_id:?} in {configured}"
            );
        }
    }
}

#[test]
fn a_refused_run_id_is_named_with_the_rule_it_breaks() {
    let refused =
        TranscriptFile::for_run(Path::new("out/{run_id}.json"), &id("../../x")).unwrap_err();

    assert_eq!(
        refused.to_string(),
        "the run id \"../../x\" can't take the place of `{run_id}` in the transcript's path: \
         it holds a `/`"
    );
}

/// The rule is the path's and not the run id's: nothing is put into a path
/// that holds no placeholder, so such a path takes any run id.
#[test]
fn a_path_that_holds_no_placeholder_takes_any_run_id() {
    for run_id in ["../../x", "a/b", "..", ".", "a\0b", FIRST] {
        let file = TranscriptFile::for_run(Path::new("out/transcript.json"), &id(run_id));

        assert_eq!(
            file.as_ref().map(TranscriptFile::path),
            Ok(Path::new("out/transcript.json")),
            "{run_id:?}"
        );
    }
}

/// What a run id may hold beside a separator is the file system's to
/// accept, so a dot that isn't the whole id, and two that aren't, are part
/// of a name.
#[test]
fn a_run_id_that_is_one_path_component_is_put_in_whatever_else_it_holds() {
    for run_id in [FIRST, "run-1", "...", ".hidden", "a..b", "run 1", "a\\b"] {
        let file = TranscriptFile::for_run(Path::new("out/{run_id}.json"), &id(run_id)).unwrap();

        assert_eq!(
            file.path(),
            Path::new(&format!("out/{run_id}.json")),
            "{run_id:?}"
        );
    }
}

#[test]
fn a_path_that_is_not_utf_8_keeps_every_byte_around_the_run_id() {
    let configured = OsStr::from_bytes(b"out/\xff-{run_id}-\xfe.json");
    let mut expected = b"out/\xff-".to_vec();
    expected.extend_from_slice(FIRST.as_bytes());
    expected.extend_from_slice(b"-\xfe.json");

    let file = TranscriptFile::for_run(Path::new(configured), &id(FIRST)).unwrap();

    assert_eq!(file.path().as_os_str().as_bytes(), expected);
}

// Writing it

#[test]
fn what_is_written_is_the_document_as_compact_json_and_a_newline() {
    let scratch = Scratch::new("compact");
    let document = document(FIRST, "You fix tests.");
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();

    assert_eq!(file.write(&document), Ok(()));

    let written = std::fs::read_to_string(file.path()).unwrap();
    assert_eq!(written, compact(&document));
    assert_eq!(written.lines().count(), 1);
    assert!(written.starts_with(r#"{"schema_version":1,"run_id":"01K5F3Z8Q4X9T2M7B6W1R0VNEC","#));
}

/// O8, the writer's half: a path that holds the run id gives each run a
/// file of its own, so the second run's transcript leaves the first's
/// where it was.
#[test]
fn two_runs_write_two_files_when_the_path_holds_the_run_id() {
    let scratch = Scratch::new("two-runs");
    let configured = scratch.path("transcript-{run_id}.json");
    let (first, second) = (
        document(FIRST, "You fix tests."),
        document(SECOND, "You write docs."),
    );

    for (run, document) in [(FIRST, &first), (SECOND, &second)] {
        TranscriptFile::for_run(&configured, &id(run))
            .unwrap()
            .write(document)
            .unwrap();
    }

    let read = |run: &str| {
        std::fs::read_to_string(scratch.path(&format!("transcript-{run}.json"))).unwrap()
    };
    assert_eq!(read(FIRST), compact(&first));
    assert_eq!(read(SECOND), compact(&second));
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 2);
}

/// A shorter transcript written over a longer one leaves nothing of the
/// longer one behind it.
#[test]
fn a_write_takes_the_place_of_whatever_the_file_held() {
    let scratch = Scratch::new("replace");
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();
    let long = document(FIRST, &"You fix tests. ".repeat(100));
    let short = document(SECOND, "Be brief.");

    file.write(&long).unwrap();
    file.write(&short).unwrap();

    assert_eq!(
        std::fs::read_to_string(file.path()).unwrap(),
        compact(&short)
    );
}

/// No directory that holds the run id can be there before the run is.
#[test]
fn the_directories_a_path_is_missing_are_made() {
    let scratch = Scratch::new("directories");
    let configured = scratch.path("out/{run_id}/transcript.json");
    let (first, second) = (
        document(FIRST, "You fix tests."),
        document(SECOND, "You write docs."),
    );

    for (run, document) in [(FIRST, &first), (SECOND, &second), (FIRST, &first)] {
        let file = TranscriptFile::for_run(&configured, &id(run)).unwrap();

        assert_eq!(file.write(document), Ok(()), "{run}");
    }

    let read = |run: &str| {
        std::fs::read_to_string(scratch.path(&format!("out/{run}/transcript.json"))).unwrap()
    };
    assert_eq!(read(FIRST), compact(&first));
    assert_eq!(read(SECOND), compact(&second));
    for run in [FIRST, SECOND] {
        let beside = std::fs::read_dir(scratch.path(&format!("out/{run}"))).unwrap();
        assert_eq!(beside.count(), 1, "nothing is left beside the transcript");
    }
}

#[test]
fn a_write_that_fails_leaves_what_the_path_held_and_nothing_beside_it() {
    let scratch = Scratch::new("whole-or-not");
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();
    let held = document(FIRST, "You fix tests.");
    file.write(&held).unwrap();

    let failed = file.replace(|mut to| {
        to.write_all(b"{\"schema_version\":1,\"run_id\":")?;
        to.flush()?;
        Err(io::Error::other("no space left"))
    });

    assert_eq!(failed.unwrap_err().to_string(), "no space left");
    assert_eq!(
        std::fs::read_to_string(file.path()).unwrap(),
        compact(&held)
    );
    let beside: Vec<_> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(beside, ["transcript.json"]);
}

#[test]
fn a_write_that_fails_where_no_file_was_leaves_none() {
    let scratch = Scratch::new("none-or-whole");
    let file = TranscriptFile::for_run(&scratch.path("out/transcript.json"), &id(FIRST)).unwrap();

    let failed = file.replace(|mut to| {
        to.write_all(b"{\"schema_version\":1,")?;
        Err(io::Error::other("no space left"))
    });

    assert!(failed.is_err());
    assert_eq!(std::fs::read_dir(scratch.path("out")).unwrap().count(), 0);
}

/// What a link at the path led to is another file than the run's
/// transcript, and the transcript takes the link's place.
#[test]
fn a_write_replaces_a_link_at_the_path_and_leaves_what_it_led_to() {
    let scratch = Scratch::new("link");
    std::fs::write(scratch.path("elsewhere.json"), "kept").unwrap();
    std::os::unix::fs::symlink(
        scratch.path("elsewhere.json"),
        scratch.path("transcript.json"),
    )
    .unwrap();
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();
    let document = document(FIRST, "You fix tests.");

    file.write(&document).unwrap();

    assert_eq!(
        std::fs::read_to_string(scratch.path("elsewhere.json")).unwrap(),
        "kept"
    );
    assert!(!file.path().is_symlink());
    assert_eq!(
        std::fs::read_to_string(file.path()).unwrap(),
        compact(&document)
    );
}

/// A name the file system takes at the path is one it takes for the
/// temporary file too, since that name holds nothing of the path's.
#[test]
fn a_name_as_long_as_the_file_system_allows_is_written() {
    let scratch = Scratch::new("long-name");
    let name = format!("{}.json", "t".repeat(250));
    let file = TranscriptFile::for_run(&scratch.path(&name), &id(FIRST)).unwrap();
    let document = document(FIRST, "You fix tests.");

    assert_eq!(file.write(&document), Ok(()));

    assert_eq!(
        std::fs::read_to_string(file.path()).unwrap(),
        compact(&document)
    );
}

fn temporary_name(number: u64) -> String {
    format!(".lablet-{}-{number}.tmp", std::process::id())
}

#[test]
fn a_temporary_file_is_made_beside_the_path_and_named_for_the_process_and_a_number() {
    let scratch = Scratch::new("temporary-name");

    let (temporary, _) = temporary_beside(&scratch.path("out.json"), NEW_FILE_MODE, || 7).unwrap();

    assert_eq!(temporary, scratch.path(&temporary_name(7)));
    assert!(temporary.is_file());
}

/// Another process can have this one's id, so a name can be taken, and
/// what took it is left as it is: a file isn't written over, and a link
/// isn't followed.
#[test]
fn a_temporary_name_that_is_taken_is_passed_over_and_what_took_it_is_left_alone() {
    let scratch = Scratch::new("temporary-taken");
    std::fs::write(scratch.path(&temporary_name(0)), "kept").unwrap();
    std::os::unix::fs::symlink(
        scratch.path("made-through-the-link"),
        scratch.path(&temporary_name(1)),
    )
    .unwrap();
    let mut numbers = 0..;

    let (temporary, _) = temporary_beside(&scratch.path("out.json"), NEW_FILE_MODE, || {
        numbers.next().unwrap()
    })
    .unwrap();

    assert_eq!(temporary, scratch.path(&temporary_name(2)));
    assert_eq!(
        std::fs::read_to_string(scratch.path(&temporary_name(0))).unwrap(),
        "kept"
    );
    assert!(scratch.path(&temporary_name(1)).is_symlink());
    assert!(!scratch.path("made-through-the-link").exists());
}

#[test]
fn a_write_whose_every_temporary_name_is_taken_fails_after_a_hundred() {
    let scratch = Scratch::new("temporary-bound");
    std::fs::write(scratch.path(&temporary_name(0)), "kept").unwrap();
    let mut tried = 0;

    let failed = temporary_beside(&scratch.path("out.json"), NEW_FILE_MODE, || {
        tried += 1;
        0
    })
    .unwrap_err();

    assert_eq!(tried, 100);
    assert_eq!(failed.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        failed.to_string(),
        "100 temporary names beside it were taken"
    );
}

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_temporary_file_is_made_no_more_open_than_the_mode_it_is_given() {
    let scratch = Scratch::new("temporary-mode");
    let (temporary, _) = temporary_beside(&scratch.path("out.json"), 0o600, || 1).unwrap();
    assert_eq!(mode(&temporary), 0o600);
}

#[test]
fn a_write_keeps_the_permissions_of_the_file_it_replaces() {
    let scratch = Scratch::new("permissions");
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();
    let document = document(FIRST, "You fix tests.");

    for kept in [0o600, 0o640, 0o444] {
        file.write(&document).unwrap();
        set_mode(file.path(), kept);

        file.write(&document).unwrap();

        assert_eq!(mode(file.path()), kept, "{kept:o}");
    }
}

/// What the path showed was the file the link led to, so the file that
/// takes the link's place shows no more than that one did.
#[test]
fn a_write_over_a_link_takes_the_permissions_of_the_file_it_led_to() {
    let scratch = Scratch::new("permissions-link");
    std::fs::write(scratch.path("elsewhere.json"), "kept").unwrap();
    set_mode(&scratch.path("elsewhere.json"), 0o600);
    std::os::unix::fs::symlink(
        scratch.path("elsewhere.json"),
        scratch.path("transcript.json"),
    )
    .unwrap();
    let file = TranscriptFile::for_run(&scratch.path("transcript.json"), &id(FIRST)).unwrap();

    file.write(&document(FIRST, "You fix tests.")).unwrap();

    assert!(!file.path().is_symlink());
    assert_eq!(mode(file.path()), 0o600);
}

#[test]
fn a_path_that_names_no_file_is_an_error() {
    let scratch = Scratch::new("no-file");
    for configured in [PathBuf::new(), "/".into(), scratch.path("out/..")] {
        let file = TranscriptFile::for_run(&configured, &id(FIRST)).unwrap();

        let refused = file.write(&document(FIRST, "You fix tests.")).unwrap_err();

        assert_eq!(
            refused,
            TranscriptWriteError::Unwritable {
                path: configured.clone(),
                reason: "the path names no file".to_owned(),
            },
            "{configured:?}"
        );
    }
    assert!(!scratch.path("out").exists(), "nothing was made on the way");
}

#[test]
fn a_path_that_cannot_be_written_is_an_error_that_names_the_path_and_says_why() {
    let scratch = Scratch::new("unwritable");
    std::fs::write(scratch.path("a-file"), "not a directory").unwrap();
    for unwritable in [
        scratch.path("a-file/transcript.json"),
        scratch.path("a-file/out/transcript.json"),
        scratch.0.clone(),
    ] {
        let file = TranscriptFile::for_run(&unwritable, &id(FIRST)).unwrap();

        let refused = file.write(&document(FIRST, "You fix tests.")).unwrap_err();

        let TranscriptWriteError::Unwritable { path, reason } = &refused else {
            panic!("{refused:?} isn't a write that failed");
        };
        assert_eq!(path, &unwritable);
        assert!(!reason.is_empty());
        assert_eq!(
            refused.to_string(),
            format!(
                "the transcript couldn't be written to {}: {reason}",
                unwritable.display()
            )
        );
    }
    assert_eq!(
        std::fs::read_to_string(scratch.path("a-file")).unwrap(),
        "not a directory",
        "a write that failed changed nothing"
    );
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
}

/// Takes `room` bytes and then fails, as a disk that fills up does.
struct Full {
    room: usize,
}

impl Write for Full {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.room == 0 {
            return Err(io::Error::other("no space left"));
        }
        let taken = bytes.len().min(self.room);
        self.room -= taken;
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Takes everything and fails when it's asked to see it through.
struct NeverFlushes;

impl Write for NeverFlushes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("the device went away"))
    }
}

#[test]
fn a_write_that_fails_part_way_is_an_error_wherever_it_fails() {
    let document = document(FIRST, "You fix tests.");
    let whole = compact(&document).len();

    for room in [0, 1, whole / 2, whole - 1] {
        let failed = render(&document, Full { room }).unwrap_err();

        assert!(failed.to_string().contains("no space left"), "{failed}");
    }
    render(&document, Full { room: whole }).unwrap();
    assert_eq!(
        render(&document, NeverFlushes).unwrap_err().to_string(),
        "the device went away"
    );
}
