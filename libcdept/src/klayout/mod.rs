//! Embedded KLayout: the `cdept-klayout` helper and how CDePT talks to it
//! (docs/KLAYOUT.md).
//!
//! KLayout is GPL; it runs in its own process (`klayout -z` executing the helper
//! script) and nothing here links it.
//!
//! - [`Engine`]: one helper, one call at a time (CLI, tests).
//! - [`Session`]: a helper per layout tab, never blocking, restarted when it stops.
//! - [`service::Service`]: a background helper for previews and exports.
//! - [`verify`]: DRC and LVS with the PDK's decks, KLayout in batch mode.
//! - [`lef`]: LEF abstract views, read-only (docs/KLAYOUT.md §9).

mod engine;
pub mod lef;
pub mod protocol;
pub mod reference;
pub mod service;
pub mod session;
pub mod setup;
pub mod verify;

pub use engine::{Engine, EngineOptions, OpenInfo, cell_layout};
pub use protocol::{Event, Input, PROTOCOL_VERSION};
pub use session::{FrameView, Reply, Session, SessionOptions, Status, Wake};
