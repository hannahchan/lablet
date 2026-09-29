use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lablet_model::{OutputCap, OutputCut, OutputKeep};
use lablet_run::{ToolErrorKind, ToolExecutor};
use lablet_tools_builtin::{BuiltinTools, ENVIRONMENT, Settings};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::json;

use crate::harness::{
    Scratch, TIMEOUT, ask, call, came_to_hold, id_in, is_there, keeping, refused, said, sent,
    within,
};

/// A variable cargo sets for every test, which is neither on the short
/// list nor added by any settings here: what a key in lablet's environment
/// is to a command.
const NOT_FOR_A_COMMAND: &str = "CARGO_MANIFEST_DIR";

/// Variables bash sets for itself, whatever it was started with.
const SET_BY_BASH: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

const MIB: u64 = 1024 * 1024;

#[tokio::test]
async fn the_result_is_what_the_command_wrote_in_the_order_written_then_the_exit_code() {
    let scratch = Scratch::new("bash-order");
    let tools = scratch.tools();

    let text = said(
        &tools,
        "bash",
        json!({ "command": "echo one; echo two >&2; echo three; echo four >&2" }),
    )
    .await;

    assert_eq!(text, "one\ntwo\nthree\nfour\nexit code: 0");
}

#[tokio::test]
async fn the_exit_code_has_a_line_of_its_own_whatever_the_command_wrote_last() {
    let scratch = Scratch::new("bash-line");
    let tools = scratch.tools();

    let no_newline = said(&tools, "bash", json!({ "command": "printf done" })).await;
    let nothing = said(&tools, "bash", json!({ "command": "true" })).await;

    assert_eq!(no_newline, "done\nexit code: 0");
    assert_eq!(nothing, "exit code: 0");
}

#[tokio::test]
async fn a_command_that_exits_with_another_code_is_a_result_that_shows_the_code() {
    let scratch = Scratch::new("bash-exit-3");
    let tools = scratch.tools();

    let output = ask(
        &tools,
        "bash",
        json!({ "command": "echo 1 test failed; exit 3" }),
    )
    .await
    .unwrap();

    assert!(!output.is_error, "a command that failed is no tool error");
    assert_eq!(output.mcp, None);
    assert_eq!(sent(output.output, None), ["1 test failed\nexit code: 3"]);
}

#[tokio::test]
async fn a_command_that_a_signal_killed_is_a_result_that_names_the_signal() {
    let scratch = Scratch::new("bash-signal");
    let tools = scratch.tools();

    let text = said(&tools, "bash", json!({ "command": "kill -9 $$" })).await;

    assert!(text.starts_with("signal: 9"), "{text}");
}

#[tokio::test]
async fn a_command_starts_in_the_root_with_nothing_to_read() {
    let scratch = Scratch::new("bash-root");
    let tools = scratch.tools();

    let text = said(&tools, "bash", json!({ "command": "pwd; cat; echo read" })).await;

    assert_eq!(
        text,
        format!("{}\nread\nexit code: 0", scratch.root().display())
    );
}

#[tokio::test]
async fn nothing_carries_from_one_command_to_the_next() {
    let scratch = Scratch::new("bash-fresh");
    let tools = scratch.tools();
    std::fs::create_dir(scratch.root().join("sub")).unwrap();

    let first = said(
        &tools,
        "bash",
        json!({ "command": "cd sub && export LEFT=behind && echo \"$LEFT in $(basename \"$PWD\")\"" }),
    )
    .await;
    let second = said(
        &tools,
        "bash",
        json!({ "command": "echo \"${LEFT-nothing} in $(basename \"$PWD\")\"" }),
    )
    .await;

    assert_eq!(first, "behind in sub\nexit code: 0");
    assert_eq!(second, "nothing in root\nexit code: 0");
}

#[tokio::test]
async fn a_command_has_the_short_list_and_what_the_settings_add_and_nothing_else_of_lablet_s() {
    let scratch = Scratch::new("bash-env");
    let tools = BuiltinTools::new(Settings {
        env: [("LABLET_ADDED".to_owned(), "by the settings".to_owned())].into(),
        ..scratch.settings()
    })
    .unwrap();
    let held = std::env::var(NOT_FOR_A_COMMAND).expect("cargo sets it for a test");
    let path = std::env::var("PATH").expect("a test has a PATH");

    let text = said(&tools, "bash", json!({ "command": "env" })).await;

    assert!(!text.contains(NOT_FOR_A_COMMAND), "{text}");
    assert!(!text.contains(&held), "{text}");
    let lines: BTreeSet<&str> = text.lines().collect();
    assert!(lines.contains(format!("PATH={path}").as_str()), "{text}");
    assert!(lines.contains("LABLET_ADDED=by the settings"), "{text}");
    let unexpected: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name)
        .filter(|name| {
            !ENVIRONMENT.contains(name) && !SET_BY_BASH.contains(name) && *name != "LABLET_ADDED"
        })
        .collect();
    assert_eq!(unexpected, [""; 0], "{text}");
}

#[tokio::test]
async fn a_variable_the_settings_add_replaces_the_one_of_lablet_s_environment() {
    let scratch = Scratch::new("bash-env-replaced");
    let tools = BuiltinTools::new(Settings {
        env: [("HOME".to_owned(), "/nowhere".to_owned())].into(),
        ..scratch.settings()
    })
    .unwrap();

    let text = said(&tools, "bash", json!({ "command": "echo $HOME" })).await;

    assert_eq!(text, "/nowhere\nexit code: 0");
}

#[tokio::test]
async fn of_a_mebibyte_a_command_wrote_four_bytes_of_each_end_are_kept_and_all_is_counted() {
    let scratch = Scratch::new("bash-mebibyte");
    let tools = scratch.tools();
    let command = format!("head -c {MIB} /dev/zero | tr '\\0' x");
    let keep = OutputKeep { head: 4, tail: 4 };

    let output = tools
        .execute(keeping(keep, call("bash", json!({ "command": command }))))
        .await
        .unwrap();

    let total = MIB + "\nexit code: 0".len() as u64;
    assert_eq!(output.output.total_bytes(), total);
    assert_eq!(output.output.kept_bytes(), 8);
    let cap = OutputCap::new(8, OutputCut::HeadTail).unwrap();
    assert_eq!(
        sent(output.output, Some(cap)),
        [
            "xxxx".to_owned(),
            format!("[truncated: {} of {total} bytes left out]", total - 8),
            "e: 0".to_owned()
        ]
    );
}

#[tokio::test]
async fn bytes_that_are_no_text_are_returned_as_the_character_that_says_so() {
    let scratch = Scratch::new("bash-bytes");
    let tools = scratch.tools();

    let text = said(&tools, "bash", json!({ "command": "printf 'a\\377b\\n'" })).await;

    assert_eq!(text, "a\u{FFFD}b\nexit code: 0");
}

#[tokio::test]
async fn a_command_is_stopped_at_the_call_s_deadline_when_that_is_the_shorter() {
    let scratch = Scratch::new("bash-deadline");
    let tools = scratch.tools();
    let deadline = Duration::from_millis(40);

    let began = Instant::now();
    let error = tools
        .execute(within(
            deadline,
            call("bash", json!({ "command": "sleep 60" })),
        ))
        .await
        .unwrap_err();
    let took = began.elapsed();

    assert_eq!(error.kind, ToolErrorKind::Timeout);
    assert_eq!(
        error.message(),
        "bash was stopped after 40ms, the longest the call could take"
    );
    assert_eq!(error.mcp, None);
    assert!(deadline <= took && took < TIMEOUT, "it took {took:?}");
}

#[tokio::test]
async fn a_command_is_stopped_at_the_executor_s_timeout_when_that_is_the_shorter() {
    let scratch = Scratch::new("bash-timeout");
    let timeout = Duration::from_millis(60);
    let tools = BuiltinTools::new(Settings {
        timeout,
        ..scratch.settings()
    })
    .unwrap();

    let began = Instant::now();
    let error = ask(&tools, "bash", json!({ "command": "sleep 60" }))
        .await
        .unwrap_err();
    let took = began.elapsed();

    assert_eq!(error.kind, ToolErrorKind::Timeout);
    assert_eq!(
        error.message(),
        "bash was stopped after 60ms, the longest the call could take"
    );
    assert!(timeout <= took && took < TIMEOUT, "it took {took:?}");
}

#[tokio::test]
async fn a_command_that_lets_go_of_its_output_and_runs_on_is_stopped_at_the_deadline() {
    let scratch = Scratch::new("bash-silent");
    let tools = scratch.tools();
    let command = "echo $$ > shell.pid; exec > /dev/null 2>&1; sleep 60";
    let shell = scratch.root().join("shell.pid");

    for deadline in [50, 200, 800, 3_200].map(Duration::from_millis) {
        let _ = std::fs::remove_file(&shell);

        let error = tools
            .execute(within(
                deadline,
                call("bash", json!({ "command": command })),
            ))
            .await
            .unwrap_err();

        assert_eq!(error.kind, ToolErrorKind::Timeout);
        if let Some(shell) = id_in(&shell) {
            assert!(!is_there(shell), "the shell outlived the call");
            return;
        }
    }
    panic!("the command never began");
}

#[tokio::test]
async fn a_call_whose_deadline_has_come_starts_nothing() {
    let scratch = Scratch::new("bash-no-time");
    let tools = scratch.tools();
    let none_left = within(
        Duration::ZERO,
        call("bash", json!({ "command": "echo started > started" })),
    );
    let timeout = BuiltinTools::new(Settings {
        timeout: Duration::ZERO,
        ..scratch.settings()
    })
    .unwrap();

    let at_the_deadline = tools.execute(none_left).await.unwrap_err();
    let at_the_timeout = ask(
        &timeout,
        "write_file",
        json!({ "path": "started", "content": "" }),
    )
    .await
    .unwrap_err();

    assert_eq!(at_the_deadline.kind, ToolErrorKind::Timeout);
    assert_eq!(
        at_the_deadline.message(),
        "bash wasn't started: the call had no time left"
    );
    assert_eq!(at_the_timeout.kind, ToolErrorKind::Timeout);
    assert_eq!(
        at_the_timeout.message(),
        "write_file wasn't started: the call had no time left"
    );
    assert_eq!(
        at_the_timeout.message(),
        "write_file wasn't started: the call had no time left"
    );
    // A command that was started would have made the file by the time one
    // that was started after it has ended.
    said(&tools, "bash", json!({ "command": "true" })).await;
    assert!(!scratch.root().join("started").exists());
}

#[tokio::test]
async fn a_shell_that_is_not_on_the_path_is_a_failure_of_the_executor_s() {
    let scratch = Scratch::new("bash-missing");
    let empty = scratch.root().join("bin");
    std::fs::create_dir(&empty).unwrap();
    let tools = BuiltinTools::new(Settings {
        env: [("PATH".to_owned(), empty.display().to_string())].into(),
        ..scratch.settings()
    })
    .unwrap();

    let error = ask(&tools, "bash", json!({ "command": "echo hello" }))
        .await
        .unwrap_err();

    assert_eq!(error.kind, ToolErrorKind::Failed);
    assert!(
        error.message().starts_with("bash couldn't be started: "),
        "{error}"
    );
}

#[tokio::test]
async fn a_process_left_in_the_background_with_its_output_elsewhere_lets_the_call_return() {
    let scratch = Scratch::new("bash-background");
    let tools = scratch.tools();

    let began = Instant::now();
    let text = said(
        &tools,
        "bash",
        json!({ "command": "sleep 60 > /dev/null 2>&1 & echo $!" }),
    )
    .await;
    let took = began.elapsed();

    let (left, rest) = text.split_once('\n').unwrap();
    let left: i32 = left.parse().unwrap();
    assert_eq!(rest, "exit code: 0");
    assert!(took < TIMEOUT, "it took {took:?}");
    assert!(is_there(left), "what the command left running was stopped");
    kill(Pid::from_raw(left), Signal::SIGKILL).unwrap();
}

#[tokio::test]
async fn a_call_that_is_given_up_leaves_no_process_of_its_command_running() {
    let scratch = Scratch::new("bash-given-up");
    let tools = Arc::new(scratch.tools());
    let (shell, child) = (
        scratch.root().join("shell.pid"),
        scratch.root().join("child.pid"),
    );
    let command = "echo $$ > shell.pid; sleep 60 & echo $! > child.pid; wait";

    let running = tokio::spawn({
        let tools = Arc::clone(&tools);
        async move { ask(&tools, "bash", json!({ "command": command })).await }
    });
    assert!(
        came_to_hold(|| id_in(&shell).is_some() && id_in(&child).is_some()).await,
        "the command never began"
    );
    let (shell, child) = (id_in(&shell).unwrap(), id_in(&child).unwrap());
    assert!(is_there(shell) && is_there(child));
    running.abort();
    let given_up = running.await.unwrap_err();

    assert!(given_up.is_cancelled());
    assert!(
        came_to_hold(|| !is_there(shell) && !is_there(child)).await,
        "the command outlived the call that was given up"
    );
}

#[tokio::test]
async fn arguments_that_do_not_fit_are_an_error_result_that_says_what_is_wrong() {
    let scratch = Scratch::new("bash-arguments");
    let tools = scratch.tools();

    for (input, says) in [
        (json!({}), "missing field `command`"),
        (json!({ "command": 3 }), "invalid type: integer `3`"),
        (
            json!({ "command": "pwd", "cwd": "/" }),
            "unknown field `cwd`, expected `command`",
        ),
        (json!("pwd"), "invalid type: string \"pwd\""),
    ] {
        let text = refused(&tools, "bash", input).await;

        assert!(
            text.starts_with("the arguments of bash don't fit: ") && text.contains(says),
            "{text}"
        );
    }
}
