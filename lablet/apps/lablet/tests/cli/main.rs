//! The binary from outside: each command run as a process in a directory of
//! its own, and what it printed, how it exited, and what it left there.

#[cfg(all(test, target_os = "linux"))]
mod dumpable;
#[cfg(test)]
mod exits;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod init;
#[cfg(test)]
mod labels;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod prompts;
#[cfg(test)]
mod summary;
