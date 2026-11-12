//! CDePT core library.
//!
//! Holds the project/library model, technology description and external tool adapters.
//! This crate has no GUI dependencies so it can back both the egui frontend and the
//! headless CLI.

pub mod cell;
pub mod entry;
pub mod library;
pub mod technology;
pub mod tools;
pub mod version;

pub use cell::Cell;
pub use entry::{Entry, EntryType};
pub use library::Library;
pub use technology::Technology;
