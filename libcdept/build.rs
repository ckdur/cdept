use std::error::Error;

use vergen_gitcl::{Emitter, GitclBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let git = GitclBuilder::all_git()?;
    Emitter::default().add_instructions(&git)?.emit()?;
    Ok(())
}
