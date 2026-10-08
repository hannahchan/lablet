//! A host builds, checks and runs a `Lablet` wherever its runtime puts the
//! task, `tokio::spawn` among them, which asks for a future that's `Send`.

use lablet::{Config, Lablet, RunRequest};
use opentelemetry::logs::NoopLoggerProvider;

/// Compiles only when `T` is `Send`.
fn send<T: Send>(_: &T) {}

/// Held when the test compiles, so the closure is never called.
#[test]
fn the_futures_of_build_check_and_run_are_send() {
    let _ = |config: Config, lablet: &mut Lablet, request: RunRequest| {
        send(&Lablet::builder(config.clone(), NoopLoggerProvider::new()).build());
        send(&lablet::build(config.clone(), NoopLoggerProvider::new()));
        send(&lablet::check(&config));
        send(&lablet.run(request));
    };
}
