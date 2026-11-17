//! Embedded xschem.
//!
//! xschem's schematic engine runs in a helper process, `cdept-xschem`, built from
//! xschem's own sources. That program is GPL; this module is the MIT side of the
//! boundary: the wire [`protocol`], the client ([`Engine`]) that starts the helper and
//! talks to it, and the generated start-up file ([`setup`]) that tells xschem where the
//! project's libraries are. [`symbol`] keeps the `schematic=` attribute of symbols right, so
//! that xschem finds a schematic that is not next to its symbol. See `docs/XSCHEM.md`.

#[cfg(unix)]
mod engine;
pub mod protocol;
pub mod setup;
pub mod symbol;
pub mod tcl;

#[cfg(unix)]
pub use engine::{Engine, EngineOptions, Instance, NetlistFormat};
