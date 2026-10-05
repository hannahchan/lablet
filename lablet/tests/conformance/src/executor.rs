//! The cases every [`ToolExecutor`] adapter is held to, whatever its tools
//! do.
//!
//! A case makes calls to the executor it's handed, and holds what comes
//! back to the port's contract. Which calls it can make depends on the
//! tools, so an adapter's test says, as a [`Subject`], and calls each case
//! from a test that leaves tokio's clock running: what an executor starts
//! takes real time, and a paused clock would reach a call's deadline before
//! the work had begun.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    Answer, KeptOutput, OutputCap, OutputCut, OutputKeep, Secrets, ToolCallEnd, ToolCallId,
    ToolCallStatus, ToolConcurrency, ToolName, ToolResultContent, ToolSource,
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

/// How long a line of what the tool writes is, its newline included.
const LINE_BYTES: u64 = 8;

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

/// The shapes a secret leaks in, which every executor is held to cutting:
/// one object each, with the value, what a tool writes and what's kept.
const LEAKS: &str = include_str!("../fixtures/leaks.json");

/// A text that holds nothing to cut, which the closing line is learnt from.
const NOTHING_TO_CUT: &str = "nothing to cut\n";

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

    /// A call to a tool whose text is `text`, which is ASCII, so that it
    /// can be cut anywhere, and ends a line. The executor may close the
    /// text with a line of its own, the same one whatever the text.
    fn writes(&self, text: &str) -> Asked;

    /// A call whose work would go on for a minute or longer, and a way to
    /// see that work. Every call of this gives work of its own, which no
    /// earlier call's deadline has touched.
    fn outlasts(&self) -> Outlasting;

    /// A call whose text is `text`, to a tool that may run beside other
    /// calls.
    fn answers(&self, text: &str) -> Asked;
}

impl Asked {
    /// The call, with `deadline` to run in, `keep` kept of its text and
    /// `secrets` cut out of it.
    fn call(self, deadline: Duration, keep: Option<OutputKeep>, secrets: Arc<Secrets>) -> ToolCall {
        ToolCall {
            id: must(ToolCallId::new("call_1"), "naming the call"),
            name: self.name,
            input: self.input,
            deadline,
            keep,
            secrets,
        }
    }
}

/// One shape a secret leaks in: the value, what a tool writes, and what the
/// executor keeps of it.
struct Leak {
    name: String,
    secret: String,
    writes: String,
    kept: String,
}

/// The leak shapes [`LEAKS`] holds.
///
/// # Panics
///
/// When the fixture isn't a list of objects with the four texts, which is a
/// fault in the fixture rather than in an executor.
fn leaks() -> Vec<Leak> {
    let fixtures: Value = must(serde_json::from_str(LEAKS), "reading the leak fixtures");
    let text = |leak: &Value, field: &str| -> String {
        leak[field]
            .as_str()
            .unwrap_or_else(|| panic!("every leak has `{field}`: {leak}"))
            .to_owned()
    };
    fixtures
        .as_array()
        .unwrap_or_else(|| panic!("the leaks are a list: {fixtures}"))
        .iter()
        .map(|leak| Leak {
            name: text(leak, "name"),
            secret: text(leak, "secret"),
            writes: text(leak, "writes"),
            kept: text(leak, "kept"),
        })
        .collect()
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

/// Text of `bytes` bytes in which every line is its own number, so that no
/// two places in it read alike and text kept from the wrong place shows.
fn numbered(bytes: u64) -> String {
    (0..bytes / LINE_BYTES).fold(String::new(), |mut text, line| {
        // Writing to a `String` can't fail, so there's no error to report.
        let _ = writeln!(text, "{line:<7}");
        text
    })
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
        let returned = executor
            .execute(call.call(deadline, None, Arc::default()))
            .await;
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
/// more, however much the tool writes: the text's own start and its own
/// end. It reports the size of all of it, and keeps everything of a call
/// that has no `keep`. A line it closes the text with is kept whole beside
/// them.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn an_executor_keeps_no_more_output_than_its_limit(subject: &dyn Subject) {
    let executor = subject.executor();
    let (written, brief) = (numbered(WRITTEN_BYTES), numbered(SHORT_BYTES));

    let long = subject
        .writes(&written)
        .call(NO_HURRY, Some(KEPT), Arc::default());
    let long = must(executor.execute(long).await, "calling the tool that writes");
    let short = subject.writes(&brief).call(NO_HURRY, None, Arc::default());
    let short = must(
        executor.execute(short).await,
        "calling the tool that writes",
    );

    assert!(!long.is_error && !short.is_error);
    let (total, kept) = (short.output.total_bytes(), short.output.kept_bytes());
    assert_eq!(kept, total, "a call with no `keep` has everything kept");
    let said = text(short.output);
    let Some(closing) = said.strip_prefix(brief.as_str()) else {
        panic!("the text of a call with no `keep` is what the tool wrote: {said:?}")
    };
    assert_eq!(
        total,
        said.len() as u64,
        "the size that a call with no `keep` reports of its text"
    );
    let closing_bytes = closing.len() as u64;

    let (total, kept) = (long.output.total_bytes(), long.output.kept_bytes());
    assert_eq!(
        total,
        WRITTEN_BYTES + closing_bytes,
        "the size of all the tool wrote"
    );
    assert_eq!(
        kept,
        KEPT.head + KEPT.tail + closing_bytes,
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
    assert_eq!(sent.truncated_from_bytes(), Some(total));
    let sent: Vec<&str> = sent
        .content()
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect();
    let line = format!(
        "[truncated: {} of {total} bytes left out]",
        WRITTEN_BYTES - KEPT.head - KEPT.tail
    );
    let mut expected = vec![&written[..4], line.as_str(), &written[written.len() - 4..]];
    if !closing.is_empty() {
        expected.push(closing);
    }
    assert_eq!(
        sent, expected,
        "the start of the text, the line, the end of the text and the closing line that the \
         model is sent"
    );
}

/// Calls to a tool whose spec says `Shared` that are made at once, none
/// waiting for another to return, each get their own answer. The loop is
/// what runs calls together, and this is all it needs of an executor, so
/// the case doesn't ask that the calls overlap.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn calls_made_at_once_to_a_shared_tool_each_get_their_own_answer(subject: &dyn Subject) {
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
        calls.spawn(async move {
            (
                call,
                executor
                    .execute(asked.call(NO_HURRY, None, Arc::default()))
                    .await,
            )
        });
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
    let returned = executor
        .execute(asked.call(NO_HURRY, None, Arc::default()))
        .await;

    match returned {
        Ok(output) => panic!(
            "a tool the executor doesn't offer ran: {:?}",
            text(output.output)
        ),
        Err(error) => assert_eq!(error.kind, ToolErrorKind::Unknown, "{error}"),
    }
}

/// An executor cuts every value of the secrets a call carries out of the
/// tool's text before it keeps any of it, so what's kept shows
/// `[secret withheld]` where a value, a line of one, its `\n`-escaped form
/// or the remainder after a scheme word was, and never the value. A copy
/// that's encoded isn't cut, so it's kept as written. The shapes are those
/// of `fixtures/leaks.json`, each cut on its own.
///
/// # Panics
///
/// Panics when that doesn't hold of `subject`, which is how a case fails.
pub async fn an_executor_cuts_the_secrets_it_is_handed(subject: &dyn Subject) {
    let executor = subject.executor();
    let closing = closing_line(subject, executor.as_ref()).await;

    for leak in leaks() {
        let secrets = Arc::new(Secrets::new([leak.secret.clone()]));
        let call = subject.writes(&leak.writes).call(NO_HURRY, None, secrets);

        let output = must(executor.execute(call).await, "calling the tool that writes");

        assert!(!output.is_error, "{}", leak.name);
        let said = text(output.output);
        assert_eq!(
            said,
            format!("{}{closing}", leak.kept),
            "{}: the text kept, with the closing line",
            leak.name
        );
        assert!(
            !said.contains(&leak.secret),
            "{}: the value was kept",
            leak.name
        );
    }
}

/// The line the executor closes a text with, learnt from a text that holds
/// nothing to cut.
async fn closing_line(subject: &dyn Subject, executor: &dyn ToolExecutor) -> String {
    let call = subject
        .writes(NOTHING_TO_CUT)
        .call(NO_HURRY, None, Arc::default());
    let output = must(executor.execute(call).await, "calling the tool that writes");
    let said = text(output.output);
    said.strip_prefix(NOTHING_TO_CUT)
        .unwrap_or_else(|| panic!("the text of a call is what the tool wrote: {said:?}"))
        .to_owned()
}

#[cfg(test)]
mod tests;
