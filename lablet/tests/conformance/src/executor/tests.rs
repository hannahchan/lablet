//! A case is what an executor fails by, so each is held to passing an
//! executor that keeps the contract and to refusing one that breaks it in
//! the one way the case is there to see.

use std::future::Future;
use std::sync::Mutex;

use lablet_model::ToolSpec;
use lablet_run::{ToolError, ToolOutput};

use super::*;

/// The one way an executor breaks the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// It keeps the contract.
    None,
    /// It keeps the contract, and closes what a tool wrote with a line of
    /// its own.
    ClosesWithALine,
    /// At the deadline it returns and leaves the work going.
    LeavesTheWorkGoing,
    /// It returns `timeout` as soon as it's called.
    GivesUpAtOnce,
    /// At the deadline it stops the work and says it failed.
    FailsAtTheDeadline,
    /// At the deadline it stops the work and returns what it has.
    AnswersAtTheDeadline,
    /// It stops the work some time after it could have.
    IsSlowToStop,
    /// It starts nothing that can be seen.
    NeverBegins,
    /// It keeps everything a tool writes.
    KeepsEverything,
    /// It feeds a call's text only while there's room for it.
    CountsWhatItKept,
    /// It keeps 8 bytes of a call that has no `keep`.
    KeepsLittleOfEveryCall,
    /// It keeps as the end of a text what followed its start.
    KeepsTheWrongEnd,
    /// It keeps as the start of a text what follows its first line.
    KeepsTheWrongStart,
    /// It offers its shared tool as one that runs alone.
    OffersNothingShared,
    /// It answers every call to its shared tool as it answered the first.
    AnswersEveryCallAlike,
    /// It says that it failed to run a tool it doesn't have.
    FailsOnAnUnknownName,
}

struct Fake {
    fault: Fault,
    /// What can be seen of the work of the last call that outlasts.
    work: Arc<Mutex<Work>>,
    /// What the shared tool answered first.
    first: Mutex<Option<String>>,
}

impl Fake {
    fn with(fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            fault,
            work: Arc::new(Mutex::new(Work::NotBegun)),
            first: Mutex::new(None),
        })
    }

    fn see(&self, work: Work) {
        *self.work.lock().unwrap() = work;
    }

    /// Sleeps for longer than any deadline, and says how the call ends at
    /// its deadline: with what, and of which kind when that's an error.
    async fn sleep(&self, deadline: Duration) -> (&'static str, Option<ToolErrorKind>) {
        let timeout = ("slept past the deadline", Some(ToolErrorKind::Timeout));
        if self.fault == Fault::GivesUpAtOnce {
            return timeout;
        }
        if self.fault != Fault::NeverBegins {
            self.see(Work::GoingOn);
        }
        if tokio::time::timeout(deadline, tokio::time::sleep(NO_HURRY * 2))
            .await
            .is_ok()
        {
            return ("slept", None);
        }
        match self.fault {
            Fault::LeavesTheWorkGoing => return timeout,
            Fault::IsSlowToStop => tokio::time::sleep(STOPS_WITHIN).await,
            _ => {}
        }
        if self.fault != Fault::NeverBegins {
            self.see(Work::Stopped);
        }
        match self.fault {
            Fault::FailsAtTheDeadline => ("was stopped", Some(ToolErrorKind::Failed)),
            Fault::AnswersAtTheDeadline => ("slept for a while", None),
            _ => timeout,
        }
    }

    fn write(&self, text: &str, keep: Option<OutputKeep>) -> ToolOutput {
        let cut = keep.is_some();
        let keep = match self.fault {
            Fault::KeepsEverything => None,
            Fault::KeepsLittleOfEveryCall => Some(KEPT),
            _ => keep,
        };
        let room = keep.map_or(usize::MAX, |keep| usize::try_from(keep.head).unwrap());
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut output = KeptOutput::new(keep);
        for (line, written) in lines.iter().enumerate() {
            let fed = match self.fault {
                Fault::CountsWhatItKept if line * written.len() >= room => break,
                Fault::KeepsTheWrongEnd if cut => lines[line.min(1)],
                Fault::KeepsTheWrongStart if cut => lines[line.max(1)],
                _ => written,
            };
            output.push(fed);
        }
        if self.fault == Fault::ClosesWithALine {
            output.close("exit code: 0");
        }
        ToolOutput {
            output,
            is_error: false,
            mcp: None,
        }
    }

    async fn echo(&self, text: &str) -> ToolOutput {
        let first = self
            .first
            .lock()
            .unwrap()
            .get_or_insert_with(|| text.to_owned())
            .clone();
        // The call isn't over when the next is made.
        tokio::time::sleep(Duration::from_millis(10)).await;
        match self.fault {
            Fault::AnswersEveryCallAlike => said(&first, None),
            _ => said(text, None),
        }
    }
}

fn said(text: &str, keep: Option<OutputKeep>) -> ToolOutput {
    let mut output = KeptOutput::new(keep);
    output.push(text);
    ToolOutput {
        output,
        is_error: false,
        mcp: None,
    }
}

fn named(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

#[async_trait::async_trait]
impl ToolExecutor for Fake {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        let spec = |name, concurrency| ToolSpec {
            name: named(name),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
            source: ToolSource::Builtin,
            concurrency,
        };
        let echo = match self.fault {
            Fault::OffersNothingShared => ToolConcurrency::Exclusive,
            _ => ToolConcurrency::Shared,
        };
        Ok(vec![
            spec("sleep", ToolConcurrency::Exclusive),
            spec("write", ToolConcurrency::Exclusive),
            spec("echo", echo),
        ])
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        match call.name.as_str() {
            "sleep" => match self.sleep(call.deadline).await {
                (says, Some(kind)) => Err(ToolError::new(kind, says)),
                (says, None) => Ok(said(says, None)),
            },
            "write" => Ok(self.write(call.input["text"].as_str().unwrap(), call.keep)),
            "echo" => Ok(self.echo(call.input["text"].as_str().unwrap()).await),
            other => Err(ToolError::new(
                match self.fault {
                    Fault::FailsOnAnUnknownName => ToolErrorKind::Failed,
                    _ => ToolErrorKind::Unknown,
                },
                format!("no tool is called {other}"),
            )),
        }
    }
}

impl Subject for Arc<Fake> {
    fn executor(&self) -> Arc<dyn ToolExecutor> {
        Arc::clone(self) as _
    }

    fn writes(&self, text: &str) -> Asked {
        Asked {
            name: named("write"),
            input: json!({ "text": text }),
        }
    }

    fn outlasts(&self) -> Outlasting {
        self.see(Work::NotBegun);
        let work = Arc::clone(&self.work);
        Outlasting {
            call: Asked {
                name: named("sleep"),
                input: json!({}),
            },
            work: Box::new(move || *work.lock().unwrap()),
        }
    }

    fn answers(&self, text: &str) -> Asked {
        Asked {
            name: named("echo"),
            input: json!({ "text": text }),
        }
    }
}

/// What the case said as it refused the executor; `None` when it passed.
async fn refusal<Case>(case: Case) -> Option<String>
where
    Case: Future<Output = ()> + Send + 'static,
{
    let panic = tokio::spawn(case).await.err()?.into_panic();
    let said = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| (*said).to_owned()));
    Some(said.expect("a case refuses in words"))
}

async fn at_the_deadline(fault: Fault) -> Option<String> {
    let fake = Fake::with(fault);
    refusal(async move { a_call_past_its_deadline_has_stopped_when_execute_returns(&fake).await })
        .await
}

async fn of_the_output(fault: Fault) -> Option<String> {
    let fake = Fake::with(fault);
    refusal(async move { an_executor_keeps_no_more_output_than_its_limit(&fake).await }).await
}

async fn of_shared_calls(fault: Fault) -> Option<String> {
    let fake = Fake::with(fault);
    refusal(
        async move { calls_made_at_once_to_a_shared_tool_each_get_their_own_answer(&fake).await },
    )
    .await
}

async fn of_an_unknown_name(fault: Fault) -> Option<String> {
    let fake = Fake::with(fault);
    refusal(async move { a_name_the_executor_does_not_offer_is_unknown(&fake).await }).await
}

/// Holds `refusal` to being one, in words that hold `words`.
fn assert_refused(refusal: Option<String>, words: &str) {
    let said = refusal.unwrap_or_else(|| panic!("the case passed, and was to say: {words}"));
    assert!(said.contains(words), "the case said: {said}");
}

#[tokio::test(start_paused = true)]
async fn an_executor_that_keeps_the_contract_passes_every_case() {
    assert_eq!(at_the_deadline(Fault::None).await, None);
    assert_eq!(of_the_output(Fault::None).await, None);
    assert_eq!(of_the_output(Fault::ClosesWithALine).await, None);
    assert_eq!(of_shared_calls(Fault::None).await, None);
    assert_eq!(of_an_unknown_name(Fault::None).await, None);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_that_leaves_the_work_going_is_refused() {
    assert_refused(
        at_the_deadline(Fault::LeavesTheWorkGoing).await,
        "and the work goes on",
    );
}

#[tokio::test(start_paused = true)]
async fn a_call_that_is_given_up_before_its_deadline_is_refused() {
    assert_refused(
        at_the_deadline(Fault::GivesUpAtOnce).await,
        "before its deadline of 50ms",
    );
}

#[tokio::test(start_paused = true)]
async fn a_call_past_its_deadline_that_ends_as_anything_but_a_timeout_is_refused() {
    assert_refused(
        at_the_deadline(Fault::FailsAtTheDeadline).await,
        "was stopped",
    );
    assert_refused(
        at_the_deadline(Fault::AnswersAtTheDeadline).await,
        "returned a result: \"slept for a while\"",
    );
}

#[tokio::test(start_paused = true)]
async fn a_call_that_returns_long_after_its_deadline_is_refused() {
    assert_refused(
        at_the_deadline(Fault::IsSlowToStop).await,
        "with a deadline of 50ms",
    );
}

#[tokio::test(start_paused = true)]
async fn work_that_never_began_holds_the_executor_to_nothing_and_is_refused() {
    assert_refused(
        at_the_deadline(Fault::NeverBegins).await,
        "the work of no call had begun by its deadline, the longest of which was 3.2s",
    );
}

#[tokio::test(start_paused = true)]
async fn an_executor_that_keeps_more_than_the_call_keeps_is_refused() {
    assert_refused(
        of_the_output(Fault::KeepsEverything).await,
        "what's kept of 1048576 bytes",
    );
}

#[tokio::test(start_paused = true)]
async fn an_executor_that_counts_only_what_it_kept_is_refused() {
    assert_refused(
        of_the_output(Fault::CountsWhatItKept).await,
        "the size of all the tool wrote",
    );
}

#[tokio::test(start_paused = true)]
async fn an_executor_that_keeps_less_than_everything_of_a_call_with_no_keep_is_refused() {
    assert_refused(
        of_the_output(Fault::KeepsLittleOfEveryCall).await,
        "a call with no `keep` has everything kept",
    );
}

#[tokio::test(start_paused = true)]
async fn an_executor_that_keeps_the_right_sizes_of_the_wrong_text_is_refused() {
    for fault in [Fault::KeepsTheWrongEnd, Fault::KeepsTheWrongStart] {
        assert_refused(
            of_the_output(fault).await,
            "the start of the text, the line, the end of the text",
        );
    }
}

#[test]
fn the_text_a_tool_is_to_write_reads_alike_in_no_two_places() {
    let written = numbered(WRITTEN_BYTES);

    assert_eq!(written.len() as u64, WRITTEN_BYTES);
    assert!(written.starts_with("0      \n1      \n"), "{written:.32}");
    assert!(written.ends_with("131070 \n131071 \n"));
    assert_eq!(numbered(SHORT_BYTES).len() as u64, SHORT_BYTES);
}

#[tokio::test(start_paused = true)]
async fn a_tool_that_runs_alone_is_refused_as_the_shared_tool() {
    assert_refused(
        of_shared_calls(Fault::OffersNothingShared).await,
        "echo is the subject's shared tool",
    );
}

#[tokio::test(start_paused = true)]
async fn calls_made_together_that_get_one_answer_are_refused() {
    assert_refused(
        of_shared_calls(Fault::AnswersEveryCallAlike).await,
        "the answer to call 0 of 8",
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_name_that_ends_as_anything_but_unknown_is_refused() {
    assert_refused(
        of_an_unknown_name(Fault::FailsOnAnUnknownName).await,
        "no tool is called no_such_tool",
    );
}
