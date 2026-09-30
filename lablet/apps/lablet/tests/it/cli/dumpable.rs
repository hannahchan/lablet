//! On Linux, a running lablet's entries in `/proc` are closed to the other
//! processes of its user, so a command it runs can't read its environment
//! there.

use std::io::{BufRead, BufReader, ErrorKind};
use std::process::Stdio;

use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab};

#[test]
fn no_other_process_of_its_user_can_read_the_environment_of_a_running_lablet() {
    let lab = Lab::new("dumpable");
    lab.write_config(ENDS, json!({}));
    // With no prompt flag, the run waits on standard input, and says so on
    // the diagnostic log first, by which time the flag is clear.
    let mut child = lab
        .lablet(&["run", "--config", CONFIG])
        .env("RUST_LOG", "lablet=info")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let waiting = stderr
        .lines()
        .map(Result::unwrap)
        .any(|line| line.contains("reading the task prompt from standard input"));
    assert!(waiting, "lablet ended before it read its prompt");

    // Only how much was read is kept, so a failure shows nothing of what
    // the environment holds.
    let read = std::fs::read(format!("/proc/{}/environ", child.id()))
        .map(|environment| environment.len())
        .map_err(|error| error.kind());
    drop(child.stdin.take());
    child.wait().unwrap();

    assert_eq!(
        read,
        Err(ErrorKind::PermissionDenied),
        "the environment of a running lablet was read; a user with CAP_SYS_PTRACE, \
         such as root, can read it whatever the flag says, so this test needs one without"
    );
}
