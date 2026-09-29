//! The cases every [`ToolExecutor`] adapter is held to, whatever its tools
//! do.
//!
//! A case makes calls to the executor it's handed, and holds what comes
//! back to the port's contract. Which calls it can make depends on the
//! tools, so an adapter's test says, as a [`Subject`], and calls each case
//! from a test that leaves tokio's clock running: what an executor starts
//! takes real time, and a paused clock would reach a call's deadline before
//! the work had begun.

use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    Answer, KeptOutput, OutputCap, OutputCut, OutputKeep, ToolCallEnd, ToolCallId, ToolCallStatus,
    ToolConcurrency, ToolName, ToolResultContent, ToolSource,
};
use lablet_run::{ToolCall, ToolErrorKind, ToolExecutor};
use serde_json::{Value, json};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::must;

/// How much the tool of the output case writes.
const WRITTEN_BYTES: u64 = 1024 * 1024;

/// What the output case has kept of it: 4 bytes of each end.
const KEPT: OutputKeep = OutputKeep { head: 4, tail: 4 };

/// How much the tool writes of which everything is kept.
const SHORT_BYTES: u64 = 64;

/// The deadlines the deadline case gives a call, one after another until
/// the work of a call had begun when its deadline came. The first is enough
/// for an executor on a machine that has little else to do, so the case is
/// quick, and the last is there for one that's busy.
const DEADLINES: [Duration; 4] = [
    Duration::from_millis(50),
    Duration::from_millis(200),
    Duration::from_millis(800),
    Duration::from_millis(3_200),
];

/// How long after its deadline a call may return, which is the time
/// stopping its work may take.
const STOPS_WITHIN: Duration = Duration::from_secs(5);

/// The deadline of a call that isn't to reach one.
const NO_HURRY: Duration = Duration::from_secs(60);

/// How many calls the case of a shared tool makes together.
const TOGETHER: usize = 8;

/// A name no executor is expected to offer.
const NO_SUCH_TOOL: &str = "no_such_tool";

/// A call a subject knows how to make: to which tool, with what arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    /// The tool to call.
    pub name: ToolName,
    /// The arguments.
    pub input: Value,
}

/// What can be seen of the work a call started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    /// Nothing shows that it ever began.
    NotBegun,
    /// It began, and it goes on.
    GoingOn,
    /// It began, and it has stopped.
    Stopped,
}

/// A call whose work outlasts any deadline a case gives it.
pub struct Outlasting {
    /// The call.
    pub call: Asked,
    /// What can be seen of the call's work, whenever it's asked.
    pub work: Box<dyn Fn() -> Work + Send + Sync>,
}

/// An executor under test, and the calls to it that the cases make.
pub trait Subject: Send + Sync {
    /// The executor. Every call gives the same one.
    fn executor(&self) -> Arc<dyn ToolExecutor>;

    /// A call to a tool whose text is `bytes` long, all of it counted, and
    /// is ASCII, so that it can be cut anywhere.
    fn writes(&self, bytes: u64) -> Asked;

    /// A call whose work would go on for a minute or longer, and a way to
    /// see that work. Every call of this gives work of its own, which no
    /// earlier call's deadline has touched.
    fn outlasts(&self) -> Outlasting;

    /// A call whose text is `text`, to a tool that may run beside other
    /// calls.
    fn answers(&self, text: &str) -> Asked;
}

impl Asked {
    /// The call, with `deadline` to run in and `keep` kept of its text.
    fn call(self, deadline: Duration, keep: Option<OutputKeep>) -> ToolCall {
        ToolCall {
            id: must(ToolCallId::new("call_1"), "naming the call"),
            name: self.name,
            input: self.input,
            deadline,
            keep,
            trace_context: None,
        }
    }
}

fn ran_ok() -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok)
}

/// The text of `output`, of which everything was kept.
fn text(output: KeptOutput) -> String {
    Answer::measured(ran_ok(), output, None, Duration::ZERO, Duration::ZERO)
        .content()
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect()
}

/// A call that runs past its deadline returns `timeout`, no sooner than the
/// deadline and only once the work it started has stopped.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails,
/// and when the work of no call had begun by its deadline, so that the case
/// held the executor to nothing.
pub async fn a_call_past_its_deadline_has_stopped_when_execute_returns(subject: &dyn Subject) {
    let executor = subject.executor();
    for deadline in DEADLINES {
        let Outlasting { call, work } = subject.outlasts();

        let began = Instant::now();
        let returned = executor.execute(call.call(deadline, None)).await;
        let work = work();
        let took = began.elapsed();

        let error = match returned {
            Ok(output) => panic!(
                "a call that outlasts its deadline of {deadline:?} returned a result: {:?}",
                text(output.output)
            ),
            Err(error) => error,
        };
        assert_eq!(error.kind, ToolErrorKind::Timeout, "{error}");
        assert!(
            took >= deadline,
            "the call was given up after {took:?}, before its deadline of {deadline:?}"
        );
        assert!(
            took < deadline + STOPS_WITHIN,
            "the call returned {took:?} after it began, with a deadline of {deadline:?}"
        );
        match work {
            // The deadline came before the work had begun, so this call
            // held the executor to nothing.
            Work::NotBegun => {}
            Work::GoingOn => {
                panic!("`execute` returned `timeout` after {took:?}, and the work goes on")
            }
            Work::Stopped => return,
        }
    }
    panic!(
        "the work of no call had begun by its deadline, the longest of which was {:?}",
        DEADLINES[DEADLINES.len() - 1]
    );
}

/// An executor keeps of a call's text what the call's `keep` says and no
/// more, however much the tool writes, reports the size of all of it, and
/// keeps everything of a call that has no `keep`.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn an_executor_keeps_no_more_output_than_its_limit(subject: &dyn Subject) {
    let executor = subject.executor();

    let long = subject.writes(WRITTEN_BYTES).call(NO_HURRY, Some(KEPT));
    let long = must(executor.execute(long).await, "calling the tool that writes");
    let short = subject.writes(SHORT_BYTES).call(NO_HURRY, None);
    let short = must(
        executor.execute(short).await,
        "calling the tool that writes",
    );

    assert!(!long.is_error && !short.is_error);
    let (total, kept) = (long.output.total_bytes(), long.output.kept_bytes());
    assert_eq!(total, WRITTEN_BYTES, "the size of all the tool wrote");
    assert_eq!(
        kept,
        KEPT.head + KEPT.tail,
        "what's kept of {total} bytes, of which {KEPT:?} is to be kept"
    );
    let cap = must(
        OutputCap::new(KEPT.head + KEPT.tail, OutputCut::HeadTail),
        "making the cap",
    );
    let sent = Answer::measured(
        ran_ok(),
        long.output,
        Some(cap),
        Duration::ZERO,
        Duration::ZERO,
    );
    assert_eq!(sent.truncated_from_bytes(), Some(WRITTEN_BYTES));
    let sizes: Vec<usize> = sent
        .content()
        .iter()
        .map(|ToolResultContent::Text(text)| text.len())
        .collect();
    let line = format!(
        "[truncated: {} of {WRITTEN_BYTES} bytes left out]",
        WRITTEN_BYTES - kept
    );
    assert_eq!(
        sizes,
        [4, line.len(), 4],
        "the start, the line and the end that the model is sent: {:?}",
        sent.content()
    );

    assert_eq!(
        (short.output.total_bytes(), short.output.kept_bytes()),
        (SHORT_BYTES, SHORT_BYTES),
        "a call with no `keep` has everything kept"
    );
    assert_eq!(text(short.output).len() as u64, SHORT_BYTES);
}

/// A tool whose spec says `Shared` answers calls that are made while
/// earlier ones have yet to return, each with its own answer.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn calls_to_a_shared_tool_are_answered_together(subject: &dyn Subject) {
    let executor = subject.executor();
    let specs = must(executor.specs().await, "listing the tools");
    let answers: Vec<String> = (0..TOGETHER)
        .map(|call| format!("the answer to call {call} of {TOGETHER}"))
        .collect();

    let mut calls = JoinSet::new();
    for (call, answer) in answers.iter().enumerate() {
        let asked = subject.answers(answer);
        let spec = specs.iter().find(|spec| spec.name == asked.name);
        assert_eq!(
            spec.map(|spec| spec.concurrency),
            Some(ToolConcurrency::Shared),
            "{} is the subject's shared tool",
            asked.name
        );
        let executor = subject.executor();
        calls.spawn(async move { (call, executor.execute(asked.call(NO_HURRY, None)).await) });
    }

    let mut answered = vec![None; TOGETHER];
    while let Some(joined) = calls.join_next().await {
        let (call, returned) = must(joined, "waiting for a call");
        let output = must(returned, "calling the shared tool");
        assert!(!output.is_error);
        answered[call] = Some(text(output.output));
    }
    let expected: Vec<Option<String>> = answers.into_iter().map(Some).collect();
    assert_eq!(answered, expected);
}

/// A call to a name the executor doesn't offer is `unknown`, which is what
/// keeps a run's tools the ones it offered when the run was built.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn a_name_the_executor_does_not_offer_is_unknown(subject: &dyn Subject) {
    let executor = subject.executor();
    let name = must(ToolName::new(NO_SUCH_TOOL), "naming a tool");
    let specs = must(executor.specs().await, "listing the tools");
    assert!(
        specs.iter().all(|spec| spec.name != name),
        "the executor offers {name}, so the case has no name to call"
    );

    let asked = Asked {
        name,
        input: json!({}),
    };
    let returned = executor.execute(asked.call(NO_HURRY, None)).await;

    match returned {
        Ok(output) => panic!(
            "a tool the executor doesn't offer ran: {:?}",
            text(output.output)
        ),
        Err(error) => assert_eq!(error.kind, ToolErrorKind::Unknown, "{error}"),
    }
}

#[cfg(test)]
mod tests;
