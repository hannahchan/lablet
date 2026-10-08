use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::*;
use crate::tests::fakes::FakeCancel;

#[test]
fn a_lablet_answers_for_the_handle_of_the_run_in_progress_and_for_none_before_any() {
    let cancellation = RunCancellation::default();
    let fired = Arc::new(FakeCancel::never());
    fired.cancel();
    assert!(!cancellation.is_cancelled());

    cancellation.set(Some(fired as _));
    assert!(cancellation.is_cancelled());

    let later = Arc::new(FakeCancel::never());
    cancellation.set(Some(Arc::clone(&later) as _));
    assert!(!cancellation.is_cancelled());
    later.cancel();
    assert!(cancellation.is_cancelled());

    cancellation.set(None);
    assert!(!cancellation.is_cancelled());
}

#[test]
fn waiting_on_the_cell_ends_only_when_the_run_in_progress_is_cancelled() {
    let cancellation = RunCancellation::default();
    let mut context = Context::from_waker(Waker::noop());

    let mut before_any = pin!(cancellation.cancelled());
    assert_eq!(before_any.as_mut().poll(&mut context), Poll::Pending);

    let handle = Arc::new(FakeCancel::never());
    cancellation.set(Some(Arc::clone(&handle) as _));
    let mut waiting = pin!(cancellation.cancelled());
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Pending);
    handle.cancel();
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Ready(()));
}

#[test]
fn a_run_nothing_asks_to_stop_is_never_cancelled_and_never_hears_of_it() {
    let never = NeverCancelled;
    let mut context = Context::from_waker(Waker::noop());

    assert!(!never.is_cancelled());
    let mut waiting = pin!(never.cancelled());
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Pending);
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Pending);
    assert!(!never.is_cancelled());
}
