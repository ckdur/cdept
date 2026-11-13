//! CDePT core library.
//!
//! Holds the project/library model, technology description and external tool adapters.
//! This crate has no GUI dependencies so it can back both the egui frontend and the
//! headless CLI.

pub mod cell;
pub mod entry;
pub mod library;
pub mod project;
pub mod technology;
pub mod template;
pub mod tools;
pub mod version;

pub use cell::Cell;
pub use entry::{Entry, Format, ViewKind};
pub use library::{Library, LibraryLayout};
pub use project::{AddLibrary, PROJECT_FILE, Project};
pub use technology::Technology;
pub use template::Template;
