use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::*;

#[test]
fn a_handle_is_not_fired_until_it_is_and_then_stays_fired() {
    let handle = CancelHandle::new();
    assert!(!handle.is_cancelled());
    assert!(!CancelHandle::default().is_cancelled());

    handle.cancel();
    assert!(handle.is_cancelled());
    handle.cancel();
    assert!(handle.is_cancelled());
}

#[test]
fn a_clone_is_the_same_handle_and_another_handle_is_not() {
    let handle = CancelHandle::new();
    let kept = handle.clone();
    let other = CancelHandle::new();

    kept.cancel();

    assert!(handle.is_cancelled());
    assert!(!other.is_cancelled());
    assert_eq!(handle, kept);
    assert_ne!(handle, other);
}

#[test]
fn the_port_answers_as_the_handle_does() {
    let handle = CancelHandle::new();
    let port: &dyn Cancellation = &handle;
    assert!(!port.is_cancelled());

    handle.cancel();
    assert!(port.is_cancelled());
}

#[test]
fn waiting_ends_when_the_handle_is_fired_and_at_once_once_it_has_been() {
    let handle = CancelHandle::new();
    let mut context = Context::from_waker(Waker::noop());

    let mut waiting = pin!(handle.cancelled());
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Pending);
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Pending);
    handle.clone().cancel();
    assert_eq!(waiting.as_mut().poll(&mut context), Poll::Ready(()));

    let mut fired = pin!(handle.cancelled());
    assert_eq!(fired.as_mut().poll(&mut context), Poll::Ready(()));
}
