//! Helpers the crate's unit tests share.

use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Polls `future` to completion on this thread, with no runtime.
///
/// A future that returns `Pending` without waking its task is never polled
/// again by the join inside `Pending::answer`, so this panics past a bound
/// rather than letting such a test hang.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    const POLLS: usize = 10_000;
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..POLLS {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
    panic!("the future didn't finish in {POLLS} polls");
}
