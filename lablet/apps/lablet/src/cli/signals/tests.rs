use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitStatus, Stdio};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use super::{Stop, on_stop};

/// Set in the environment of the child process the tests below start, to
/// the number of signals it will be sent.
const CHILD: &str = "LABLET_TEST_STOP_SIGNALS";

/// What starts each line the child says, so the test harness's own lines
/// can't be taken for one.
const SAYS: &str = "stop signals: ";

/// The child's side, which does nothing unless the tests below started it:
/// it catches the stop signals, says so, and says which signal each one it
/// catches is.
#[test]
fn a_child_process_says_which_stop_signals_it_caught() {
    let Some(count) = std::env::var_os(CHILD) else {
        return;
    };
    let count: usize = count.to_str().unwrap().parse().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (caught, mut stops) = tokio::sync::mpsc::unbounded_channel();
        let _watching = on_stop(move |stop| caught.send(stop).unwrap()).unwrap();
        say("ready");
        for _ in 0..count {
            let stop = stops.recv().await.expect("the watch ended");
            say(&format!("caught {stop}"));
        }
    });
}

fn say(line: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{SAYS}{line}").unwrap();
    stdout.flush().unwrap();
}

/// Starts the child, sends it each of `signals` once it has said it caught
/// the one before, and returns what it said and how it exited.
fn sent(signals: &[Signal]) -> (Vec<String>, ExitStatus) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cli::signals::tests::a_child_process_says_which_stop_signals_it_caught",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, signals.len().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut said = stdout
        .lines()
        .map(Result::unwrap)
        .filter_map(|line| Some(line.split_once(SAYS)?.1.to_owned()));
    let mut heard = Vec::new();
    let mut next = |heard: &mut Vec<String>| {
        let line = said.next().expect("the child ended before it said more");
        heard.push(line);
    };

    next(&mut heard);
    for signal in signals {
        kill(pid, *signal).unwrap();
        next(&mut heard);
    }
    (heard, child.wait().unwrap())
}

#[test]
fn ctrl_c_and_sigterm_each_reach_the_callback_every_time_and_leave_the_process_running() {
    let (said, exited) = sent(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGINT]);

    assert_eq!(
        said,
        ["ready", "caught SIGINT", "caught SIGTERM", "caught SIGINT"]
    );
    assert!(exited.success(), "{exited}");
}

#[test]
fn a_stop_signal_is_named_as_the_system_names_it() {
    assert_eq!(Stop::Interrupt.to_string(), "SIGINT");
    assert_eq!(Stop::Terminate.to_string(), "SIGTERM");
}
