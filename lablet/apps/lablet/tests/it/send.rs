//! A host builds, checks and runs a `Lablet` wherever its runtime puts the
//! task, `tokio::spawn` among them, which asks for a future that's `Send`,
//! and shares one `Otel` between such tasks, which asks that it's `Sync`
//! too.

use lablet::{Config, Lablet, Otel, RunRequest};

/// Compiles only when `T` is `Send`.
fn send<T: Send>(_: &T) {}

/// Compiles only when `T` is `Send` and `Sync`.
fn send_sync<T: Send + Sync + 'static>() {}

/// Held when the test compiles, so the closure is never called.
#[test]
fn the_futures_of_build_check_and_run_are_send() {
    send_sync::<Otel>();
    send_sync::<lablet::Builder>();
    let _ = |config: Config, lablet: &mut Lablet, request: RunRequest| {
        send(&Lablet::builder(config.clone(), Otel::noop()).build());
        send(&lablet::build(config.clone(), Otel::noop()));
        send(&lablet::check(&config));
        send(&lablet.run(request));
    };
}
