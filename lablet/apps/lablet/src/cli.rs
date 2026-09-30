//! The command line's decisions, each a function a test calls, so that
//! `main.rs` is left only the effects that act on them.

pub(crate) mod args;
pub(crate) mod diagnostics;
pub(crate) mod init;
pub(crate) mod refusal;
pub(crate) mod report;
pub(crate) mod request;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the library has no way to stop a run yet, and signals caught before it has would leave Ctrl-C doing nothing"
    )
)]
pub(crate) mod signals;
