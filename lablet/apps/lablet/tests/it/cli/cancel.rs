//! Ctrl-C and `SIGTERM`, which stop a run of the binary as a fired handle
//! stops one of the library's (C14).

use std::time::{Duration, Instant};

use serde_json::json;

use super::harness::Lab;
use crate::harness::{Traced, json_of};
use crate::key;

/// The grace period a container's stop allows by default, which a stopped
/// run returns inside (spec §1).
const GRACE: Duration = Duration::from_secs(10);

/// A call of `bash` whose command sends lablet `signal`, its parent, and
/// then waits far longer than the call may take, and a response that ends
/// the run, which a run that went on would reach.
fn signals_then_waits(signal: &str) -> String {
    format!(
        "
- response:
    content:
      - tool_use: {{ id: call_1, name: bash, input: {{ json: {{ command: 'kill -{signal} $PPID; sleep 60' }} }} }}
    finish: tool_use
- response:
    content:
      - text: The command ran to its end.
    finish: end_turn
"
    )
}

#[test]
fn a_stop_signal_during_a_tool_call_stops_the_call_and_the_run_with_its_record_written() {
    for signal in ["TERM", "INT"] {
        let lab = Lab::new(&format!("cancel-{signal}"));
        let mut builtin = lab.builtin(&["bash"]);
        // Were the signal not to stop the call, the call would time out
        // well before its command ended, and the run would go on to
        // complete.
        builtin["timeout"] = json!("10s");
        lab.write_config(
            &signals_then_waits(signal),
            json!({
                "run": { "transcript_path": "transcript.json" },
                "tools": { "builtin": builtin },
            }),
        );

        // The spec bounds the time from the stop to the run's return. This
        // bounds the whole run, which must finish, so the start and the
        // build count too; they take a fraction of a second, which leaves
        // a loaded runner a wide margin.
        let started = Instant::now();
        let run = lab.run_config(&["--run-id", "stopped"]);
        let took = started.elapsed();

        assert!(took < GRACE, "{signal}: {took:?}");
        assert_eq!(run.code, Some(2), "{signal}: {run:?}");
        let outcome = run.outcome();
        assert_eq!(outcome["stop_reason"], json!("cancelled"), "{signal}");
        assert_eq!(
            (&outcome["turns"], &outcome["tool_calls"]),
            (&json!(1), &json!(1))
        );
        let lines = run.stderr_lines();
        assert_eq!(lines.len(), 1, "{signal}: {run:?}");
        assert!(
            lines[0].starts_with("cancelled: 1 turn, "),
            "{signal}: {run:?}"
        );

        let transcript = json_of(&lab.at("transcript.json"));
        assert_eq!(transcript["run_id"], json!("stopped"));
        assert_eq!(
            transcript["turns"][0]["tool_calls"][0]["status"],
            json!({ "ran": { "source": "builtin", "ended": "cancelled" } }),
            "{signal}"
        );
        let exported = lab.exported();
        let traced = Traced::of(&exported, "stopped");
        assert_eq!(
            traced.wide().attributes[key::LABLET_RUN_STOP_REASON],
            json!("cancelled"),
            "{signal}"
        );
        assert_eq!(
            traced.chats().len(),
            1,
            "{signal}: a provider call followed"
        );
    }
}
