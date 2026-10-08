//! The binary from outside, and the command line's composition through its
//! library target: a config in, a run out, and what each run left in its
//! files and sent to a receiver.

#[cfg(test)]
mod cli;
#[cfg(test)]
mod export;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod invariants;
#[cfg(test)]
mod key;
#[cfg(test)]
mod network;
#[cfg(test)]
mod otlp;
#[cfg(test)]
mod smoke;
#[cfg(test)]
mod wide;
#[cfg(test)]
mod wide_checks;
