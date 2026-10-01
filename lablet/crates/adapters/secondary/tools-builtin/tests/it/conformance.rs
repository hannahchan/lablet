//! The cases every executor is held to, of the built-in tools.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lablet_conformance::executor::{
    Asked, Outlasting, Subject, Work, a_call_past_its_deadline_has_stopped_when_execute_returns,
    a_name_the_executor_does_not_offer_is_unknown, an_executor_cuts_the_secrets_it_is_handed,
    an_executor_keeps_no_more_output_than_its_limit,
    calls_made_at_once_to_a_shared_tool_each_get_their_own_answer,
};
use lablet_run::ToolExecutor;
use lablet_tools_builtin::BuiltinTools;
use serde_json::json;

use crate::harness::{Root, id_in, is_there, name};

/// Which tool the text of the output case comes from.
#[derive(Clone, Copy)]
enum Writer {
    /// `bash`, of a command that writes it and then exits, which `bash`
    /// closes the text with a line about.
    Bash,
    /// `read_file`, of a file that holds it.
    ReadFile,
}

struct Builtin {
    tools: Arc<BuiltinTools>,
    scratch: Root,
    writer: Writer,
    /// How many files the subject has named, so that each has a name of
    /// its own.
    files: AtomicU32,
}

impl Builtin {
    fn under(test: &str, writer: Writer) -> Self {
        let scratch = Root::new(test);
        Self {
            tools: Arc::new(scratch.tools()),
            scratch,
            writer,
            files: AtomicU32::new(0),
        }
    }

    fn next(&self) -> u32 {
        self.files.fetch_add(1, Ordering::Relaxed)
    }
}

/// What the process table says of the shell and of the child it started,
/// whose ids the command wrote to the two files.
fn work_of(shell: &Path, child: &Path) -> Work {
    match (id_in(shell), id_in(child)) {
        (Some(shell), Some(child)) if is_there(shell) || is_there(child) => Work::GoingOn,
        (Some(_), Some(_)) => Work::Stopped,
        _ => Work::NotBegun,
    }
}

impl Subject for Builtin {
    fn executor(&self) -> Arc<dyn ToolExecutor> {
        Arc::clone(&self.tools) as _
    }

    fn writes(&self, text: &str) -> Asked {
        let file = format!("written-{}.txt", self.next());
        self.scratch.holds(&file, text);
        match self.writer {
            Writer::Bash => Asked {
                name: name("bash"),
                input: json!({ "command": format!("cat {file}") }),
            },
            Writer::ReadFile => Asked {
                name: name("read_file"),
                input: json!({ "path": file }),
            },
        }
    }

    fn outlasts(&self) -> Outlasting {
        let work = self.next();
        let (shell, child) = (format!("shell-{work}.pid"), format!("child-{work}.pid"));
        let command = format!("echo $$ > {shell}; sleep 60 & echo $! > {child}; wait");
        let (shell, child) = (
            self.scratch.root().join(shell),
            self.scratch.root().join(child),
        );
        Outlasting {
            call: Asked {
                name: name("bash"),
                input: json!({ "command": command }),
            },
            work: Box::new(move || work_of(&shell, &child)),
        }
    }

    fn answers(&self, text: &str) -> Asked {
        let file = format!("answer-{}.txt", self.next());
        self.scratch.holds(&file, text);
        Asked {
            name: name("read_file"),
            input: json!({ "path": file }),
        }
    }
}

#[tokio::test]
async fn a_command_past_its_deadline_has_stopped_with_its_child_when_execute_returns() {
    let subject = Builtin::under("conformance-deadline", Writer::Bash);

    a_call_past_its_deadline_has_stopped_when_execute_returns(&subject).await;
}

#[tokio::test]
async fn no_more_is_kept_of_what_a_command_writes_than_the_call_keeps() {
    let subject = Builtin::under("conformance-output-bash", Writer::Bash);

    an_executor_keeps_no_more_output_than_its_limit(&subject).await;
}

#[tokio::test]
async fn no_more_is_kept_of_what_a_file_holds_than_the_call_keeps() {
    let subject = Builtin::under("conformance-output-read-file", Writer::ReadFile);

    an_executor_keeps_no_more_output_than_its_limit(&subject).await;
}

#[tokio::test]
async fn reads_that_are_made_together_are_each_answered_with_their_own_file() {
    let subject = Builtin::under("conformance-shared", Writer::ReadFile);

    calls_made_at_once_to_a_shared_tool_each_get_their_own_answer(&subject).await;
}

#[tokio::test]
async fn a_name_that_is_no_built_in_tool_s_is_unknown() {
    let subject = Builtin::under("conformance-unknown", Writer::Bash);

    a_name_the_executor_does_not_offer_is_unknown(&subject).await;
}

#[tokio::test]
async fn a_secret_a_command_prints_in_any_leak_shape_is_cut() {
    let subject = Builtin::under("conformance-cut-bash", Writer::Bash);

    an_executor_cuts_the_secrets_it_is_handed(&subject).await;
}

#[tokio::test]
async fn a_secret_a_file_holds_in_any_leak_shape_is_cut() {
    let subject = Builtin::under("conformance-cut-read-file", Writer::ReadFile);

    an_executor_cuts_the_secrets_it_is_handed(&subject).await;
}
