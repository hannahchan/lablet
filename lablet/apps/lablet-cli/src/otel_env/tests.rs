//! The seam over environments the tests state, with the warnings caught by
//! a `tracing` subscriber of the test's own: the same helper as the parsing
//! tests of `lablet-otel-env`, whose test module no other crate can reach.

use std::ffi::OsString;
use std::io;
use std::sync::{Arc, Mutex};

use super::*;

/// What `read` comes to over an environment that holds `held` and nothing
/// else, and each warning it logged, one line each.
pub(crate) fn warned<T>(
    held: &[(&str, &str)],
    read: impl FnOnce(&Variables<'_>) -> T,
) -> (T, Vec<String>) {
    let held: Vec<(String, OsString)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
        .collect();
    within(&held, read)
}

fn within<T>(
    held: &[(String, OsString)],
    read: impl FnOnce(&Variables<'_>) -> T,
) -> (T, Vec<String>) {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, value)| value.clone())
    };
    let written = Written::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(written.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_ansi(false)
        .finish();
    let read = tracing::subscriber::with_default(subscriber, || read(&Variables(&env)));
    let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
    (read, text.lines().map(str::to_owned).collect())
}

/// What the subscriber writes, kept.
#[derive(Debug, Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Written {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn every_variable_is_read_from_the_environment_it_is_given() {
    let (read, warnings) = warned(&[("OTEL_TRACES_SAMPLER", "always_off")], |variables| {
        Sdk::read(variables)
    });
    let env = |name: &str| (name == "OTEL_TRACES_SAMPLER").then(|| OsString::from("always_off"));

    assert_eq!(OtelEnv::read(&env).sdk, read);
    assert!(warnings.is_empty());
    assert_ne!(read, Sdk::default());
}
