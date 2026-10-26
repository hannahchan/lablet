//! Integration tests of `lablet-provider-anthropic`, through its public
//! surface only: a target and a provider at a wiremock server that stands
//! in for the API, called directly where the mapping is the question and
//! through the loop where what the loop does with it is.

#[cfg(test)]
mod bound;
#[cfg(test)]
mod cache;
#[cfg(test)]
mod context;
#[cfg(test)]
mod deadline;
#[cfg(test)]
mod errors;
#[cfg(test)]
mod happy;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod prefix;
#[cfg(test)]
mod redirect;
#[cfg(test)]
mod replay;
#[cfg(test)]
mod secrets;
#[cfg(test)]
mod settings;
#[cfg(test)]
mod target;
#[cfg(test)]
mod thinking;
