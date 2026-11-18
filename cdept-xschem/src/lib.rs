//! The xschem engine linked into the `cdept-xschem` helper program.
//!
//! This crate compiles xschem's C sources (GPL-2.0-or-later) and is therefore GPL
//! itself. Only the helper program and its own tests and tools use it; CDePT talks to
//! the helper over a socket (`libcdept::xschem`).

pub mod start;
pub mod xschem;
