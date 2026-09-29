use std::sync::Mutex;

use lablet_model::RunId;
use lablet_run::EventKind;

use super::*;

/// What each observer heard, under the observer's name, in the order it
/// was heard.
type Heard = Arc<Mutex<Vec<(&'static str, String)>>>;

/// Keeps what it's told in a list it shares with the others, under its own
/// name, and answers a call with the span it was given.
struct Told {
    name: &'static str,
    heard: Heard,
    span: Option<&'static str>,
}

#[async_trait::async_trait]
impl RunObserver for Told {
    async fn on(&self, event: RunEvent) {
        self.heard
            .lock()
            .unwrap()
            .push((self.name, format!("{}:{}", event.run_id, event.kind.name())));
    }

    fn trace_context(&self, _: &ToolCallId) -> Option<TraceContext> {
        self.span.map(|traceparent| TraceContext {
            traceparent: traceparent.to_owned(),
            tracestate: None,
        })
    }
}

fn observers(spans: &[(&'static str, Option<&'static str>)]) -> (FanOut, Heard) {
    let heard = Heard::default();
    let observers = spans
        .iter()
        .map(|&(name, span)| {
            Arc::new(Told {
                name,
                heard: Arc::clone(&heard),
                span,
            }) as Arc<dyn RunObserver>
        })
        .collect();
    (FanOut::new(observers), heard)
}

fn turn(run: &str, turn: u32) -> RunEvent {
    RunEvent {
        run_id: RunId::new(run).unwrap(),
        kind: EventKind::TurnStarted { turn },
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn every_observer_is_told_every_event_in_the_order_they_were_given() {
    let (fanout, heard) = observers(&[("first", None), ("second", None), ("third", None)]);

    block_on(async {
        fanout.on(turn("run-1", 1)).await;
        fanout.on(turn("run-2", 1)).await;
    });

    assert_eq!(
        *heard.lock().unwrap(),
        [
            ("first", "run-1:TurnStarted".to_owned()),
            ("second", "run-1:TurnStarted".to_owned()),
            ("third", "run-1:TurnStarted".to_owned()),
            ("first", "run-2:TurnStarted".to_owned()),
            ("second", "run-2:TurnStarted".to_owned()),
            ("third", "run-2:TurnStarted".to_owned()),
        ]
    );
}

#[test]
fn no_observers_is_nothing_to_tell_and_no_span_to_propagate() {
    let (fanout, heard) = observers(&[]);

    block_on(fanout.on(turn("run-1", 1)));

    assert!(heard.lock().unwrap().is_empty());
    assert_eq!(
        fanout.trace_context(&ToolCallId::new("call_1").unwrap()),
        None
    );
}

#[test]
fn a_call_propagates_the_span_of_the_first_observer_that_has_one() {
    let call = ToolCallId::new("call_1").unwrap();
    let span = |traceparent: &str| {
        Some(TraceContext {
            traceparent: traceparent.to_owned(),
            tracestate: None,
        })
    };

    let (fanout, _) = observers(&[
        ("first", None),
        ("second", Some("00-a-b-01")),
        ("third", Some("00-c-d-01")),
    ]);
    assert_eq!(fanout.trace_context(&call), span("00-a-b-01"));

    let (fanout, _) = observers(&[("first", Some("00-e-f-01")), ("second", Some("00-a-b-01"))]);
    assert_eq!(fanout.trace_context(&call), span("00-e-f-01"));

    let (fanout, _) = observers(&[("first", None), ("second", None)]);
    assert_eq!(fanout.trace_context(&call), None);
}
