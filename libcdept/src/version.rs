//! Build version information (git data comes from `vergen-gitcl` in `build.rs`).

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: &str = env!("VERGEN_GIT_SHA");
pub const GIT_DIRTY: &str = env!("VERGEN_GIT_DIRTY");

/// Short git hash, with a `*` suffix if the tree was dirty at build time.
pub fn git_short() -> String {
    let sha: String = GIT_SHA.chars().take(7).collect();
    if GIT_DIRTY == "true" {
        format!("{sha}*")
    } else {
        sha
    }
}

/// e.g. `0.1.0 (fe588ec)`.
pub fn long_version() -> String {
    format!("{VERSION} ({})", git_short())
}
