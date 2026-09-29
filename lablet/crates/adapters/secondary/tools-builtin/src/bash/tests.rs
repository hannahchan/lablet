//! The wait for a group that was killed, on a clock the tests hold still.
//! Most tests say what the look for the group sees; one starts a process in
//! a group of its own, so the look that runs is held too.

use std::cell::Cell;
use std::os::unix::process::ExitStatusExt as _;

use lablet_run::ToolErrorKind;
use tokio::time::Instant;

use super::*;

const LIMIT: Duration = Duration::from_millis(40);

fn terms() -> Terms {
    Terms {
        keep: None,
        limit: LIMIT,
    }
}

fn exited() -> ExitStatus {
    ExitStatus::from_raw(0)
}

/// What looks for a group that's there for the first `looks` looks at it,
/// and counts them in `looked`.
fn there_for(looks: u32, looked: &Cell<u32>) -> impl Fn() -> bool {
    move || {
        looked.set(looked.get() + 1);
        looked.get() <= looks
    }
}

#[tokio::test(start_paused = true)]
async fn a_group_that_is_there_for_some_looks_is_waited_for_and_went() {
    let looked = Cell::new(0);
    let began = Instant::now();

    let went = went(there_for(7, &looked)).await;

    assert!(went);
    assert_eq!(looked.get(), 8, "it's looked at until it has gone");
    assert_eq!(began.elapsed(), LOOKS_EVERY * 7);
}

#[tokio::test(start_paused = true)]
async fn a_group_that_has_gone_already_is_not_waited_for() {
    let began = Instant::now();

    let went = went(|| false).await;

    assert!(went);
    assert_eq!(began.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn a_group_that_never_goes_is_given_up_when_its_time_is_over() {
    let looked = Cell::new(0);
    let began = Instant::now();

    let went = went(there_for(u32::MAX, &looked)).await;

    assert!(!went);
    assert_eq!(began.elapsed(), GONE_WITHIN);
    assert!(looked.get() > 1, "it was looked at more than once");
}

#[tokio::test(start_paused = true)]
async fn a_command_that_was_killed_is_a_timeout_once_what_it_started_went() {
    let looked = Cell::new(0);
    let began = Instant::now();

    let error = ended_by_the_kill(terms(), Ok(exited()), went(there_for(3, &looked))).await;

    assert_eq!(error.kind, ToolErrorKind::Timeout);
    assert_eq!(
        error.message(),
        "bash was stopped after 40ms, the longest the call could take"
    );
    assert_eq!(began.elapsed(), LOOKS_EVERY * 3, "the call waited for it");
}

#[tokio::test(start_paused = true)]
async fn a_command_that_was_killed_is_a_failure_when_what_it_started_never_went() {
    let began = Instant::now();

    let error = ended_by_the_kill(terms(), Ok(exited()), went(|| true)).await;

    assert_eq!(error.kind, ToolErrorKind::Failed);
    assert_eq!(
        error.message(),
        "the command was killed after 40ms, and 5s later a process it started hadn't gone"
    );
    assert_eq!(began.elapsed(), GONE_WITHIN);
}

#[tokio::test(start_paused = true)]
async fn a_shell_that_could_not_be_waited_for_is_a_failure_and_nothing_is_looked_for() {
    let looked = Cell::new(0);
    let waited = Err(std::io::Error::other("no child"));

    let error = ended_by_the_kill(terms(), waited, went(there_for(3, &looked))).await;

    assert_eq!(error.kind, ToolErrorKind::Failed);
    assert_eq!(
        error.message(),
        "the command was killed after 40ms, and the shell couldn't be waited for: no child"
    );
    assert_eq!(looked.get(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_real_group_is_waited_for_while_it_is_there_and_went_once_it_has_gone() {
    use std::os::unix::process::CommandExt as _;

    let mut sleep = std::process::Command::new("sleep")
        .arg("60")
        .process_group(0)
        .spawn()
        .unwrap();
    // Dropped while it runs, it kills the sleep, so a test that fails
    // leaves nothing running.
    let mut group = Group {
        id: Pid::from_raw(i32::try_from(sleep.id()).unwrap()),
        running: true,
    };
    let began = Instant::now();

    assert!(!group.went().await, "a group that's there hasn't gone");
    assert_eq!(began.elapsed(), GONE_WITHIN);
    assert!(group.running);

    group.kill();
    sleep.wait().unwrap();

    assert!(group.went().await);
    assert!(!group.running);
}
