//! The one place lablet reads the process's environment: the command line's
//! input module. It fills the file's `${VAR}`s, through a function the
//! config's fill is handed, and takes the environment every process a run
//! starts begins from, once, as the base the composition hands in. The
//! seam's reads of the OpenTelemetry variables go through the same
//! function. Nothing else in lablet reads a variable: the library takes a
//! `Config` its host filled and the environment its host hands in, and the
//! root `clippy.toml` holds every other crate to that.

use std::ffi::OsString;

/// What the variable `name` holds in the process's environment, for the
/// config's fill and the seam.
#[must_use]
pub fn variable(name: &str) -> Option<OsString> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the input module is where the command line reads its environment"
    )]
    std::env::var_os(name)
}

/// The process's whole environment, as the base every process a run
/// starts begins from.
#[must_use]
pub fn process_environment() -> Vec<(OsString, OsString)> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the input module is where the command line reads its environment"
    )]
    std::env::vars_os().collect()
}
