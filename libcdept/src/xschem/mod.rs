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
pub mod frame;
pub mod input;
pub mod protocol;
#[cfg(unix)]
pub mod service;
#[cfg(unix)]
pub mod session;
pub mod setup;
pub mod symbol;
pub mod tcl;

#[cfg(unix)]
pub use engine::{
    Engine, EngineOptions, Hierarchy, HierarchyCell, HierarchyInstance, Instance, NetlistFormat,
};
#[cfg(unix)]
pub use service::{Pending, Service};
#[cfg(unix)]
pub use session::{Session, SessionOptions, Status};
