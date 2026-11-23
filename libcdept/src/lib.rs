//! CDePT core library.
//!
//! Holds the project/library model, technology description and external tool adapters.
//! This crate has no GUI dependencies so it can back both the egui frontend and the
//! headless CLI.

pub mod batch;
pub mod category;
pub mod cell;
pub mod container;
pub mod entry;
pub mod formats;
pub mod hierarchy;
pub mod import;
pub mod klayout;
pub mod layout;
pub mod library;
pub mod link;
pub mod project;
pub mod project_edit;
pub mod refactor;
pub mod search;
pub mod sim;
pub mod tech;
pub mod technology;
pub mod template;
pub mod tools;
pub mod trash;
pub mod vcs;
pub mod version;
pub mod watch;
pub mod xschem;

pub use cell::Cell;
pub use entry::{Entry, Format, ViewKind};
pub use library::{Library, LibraryLayout};
pub use project::{AddLibrary, PROJECT_FILE, Project, Settings};
pub use technology::{Pdk, Resolved, Source, Technology};
pub use template::Template;
