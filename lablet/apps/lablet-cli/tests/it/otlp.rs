//! The network exporter through the binary: what a port nothing listens on
//! costs a run and the process.

use std::time::{Duration, Instant};

use lablet_conformance::receiver::Receiver;
use serde_json::json;

use crate::harness::{ENDS, Lab};

/// O3: a closed port changes neither the outcome nor the exit code, the
/// failure is on the diagnostic log, and the process is gone within five
/// seconds of the run's end.
#[test]
fn a_port_nothing_listens_on_changes_nothing_and_the_process_exits_within_five_seconds() {
    let closed = Receiver::closed();
    let lab = Lab::new("otlp-closed-port");
    lab.write_config(
        ENDS,
        json!({ "telemetry": { "otlp": { "enabled": true, "endpoint": format!("http://{closed}") } } }),
    );
    let plain = Lab::new("otlp-closed-port-plain");
    plain.write_config(ENDS, json!({}));

    let began = Instant::now();
    let ran = lab.run_config(&[]);
    let cost = began.elapsed();
    let expected = plain.run_config(&[]);

    assert_eq!(ran.code, Some(0), "{ran:?}");
    assert_eq!(expected.code, Some(0), "{expected:?}");
    let (outcome, plain_outcome) = (ran.outcome(), expected.outcome());
    assert_eq!(outcome["stop_reason"], "completed");
    assert_eq!(outcome["stop_reason"], plain_outcome["stop_reason"]);
    assert_eq!(outcome["usage"], plain_outcome["usage"]);
    for said in [
        "BatchSpanProcessor.ExportError",
        "the run's telemetry wasn't exported whole",
    ] {
        assert!(ran.stderr.contains(said), "{said}: {}", ran.stderr);
    }
    assert!(
        !ran.stderr.contains(&closed.to_string()),
        "the endpoint is in no diagnostic line: {}",
        ran.stderr
    );
    assert!(cost < Duration::from_secs(5), "the process took {cost:?}");
    assert_eq!(
        lab.exported().records_of("lablet.run").len(),
        1,
        "the file is whole whatever the port did"
    );
}
