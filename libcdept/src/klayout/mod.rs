//! Embedded KLayout: the `cdept-klayout` helper and how CDePT talks to it
//! (docs/KLAYOUT.md).
//!
//! KLayout is GPL; it runs in its own process (`klayout -z` executing the helper
//! script) and nothing here links it.
//!
//! - [`Engine`]: one helper, one call at a time (CLI, tests).
//! - [`Session`]: a helper per layout tab, never blocking, restarted when it stops.
//! - [`service::Service`]: a background helper for previews and exports.

mod engine;
pub mod protocol;
pub mod service;
pub mod session;
pub mod setup;

pub use engine::{Engine, EngineOptions, OpenInfo, cell_layout};
pub use protocol::{Event, Input, PROTOCOL_VERSION};
pub use session::{FrameView, Reply, Session, SessionOptions, Status, Wake};
