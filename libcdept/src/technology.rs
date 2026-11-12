//! Technology (PDK) description. Filled in during Phase 2.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Technology {
    pub name: String,
    pub file: PathBuf,
}
