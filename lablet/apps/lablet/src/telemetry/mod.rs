//! The composition root's telemetry, typed: the root span and the wide
//! event, as structs generated from the registry, and the domain's closed
//! sets spelt as the registry spells them.
//!
//! [`generated`] is the module `cargo xtask weaver generate` writes from the
//! registry folder `lablet/telemetry/registry/apps/lablet/`. It's public so a
//! library caller may read the keys and the structs of the signals a run
//! emits, as it may `lablet_run::telemetry`'s; it's no extension point.
//! Nothing plugs into a `Lablet`'s telemetry, and nothing here is implemented
//! by a caller.

pub mod generated;
pub(crate) mod spellings;
