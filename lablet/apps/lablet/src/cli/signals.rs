//! The signals that ask a run to stop: Ctrl-C's, and `SIGTERM`, which is
//! what a container is sent when it's stopped.

use std::fmt;
use std::io;

use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;

/// A signal that asks lablet to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// `SIGINT`, which Ctrl-C sends.
    Interrupt,
    /// `SIGTERM`.
    Terminate,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        })
    }
}

/// Calls `fire` with each stop signal the process is sent, from a task on
/// the runtime this is called on, until the task is aborted.
///
/// The signals are caught before this returns, so one sent after it has
/// returned reaches `fire` and doesn't end the process. They stay caught
/// for as long as the process lives, even once the task has ended.
///
/// # Errors
///
/// Returns the error of a signal that couldn't be caught.
///
/// # Panics
///
/// When it isn't called on a tokio runtime.
pub(crate) fn on_stop(mut fire: impl FnMut(Stop) + Send + 'static) -> io::Result<JoinHandle<()>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(tokio::spawn(async move {
        loop {
            let stop = tokio::select! {
                Some(()) = interrupt.recv() => Stop::Interrupt,
                Some(()) = terminate.recv() => Stop::Terminate,
                else => return,
            };
            fire(stop);
        }
    }))
}

#[cfg(test)]
mod tests;
