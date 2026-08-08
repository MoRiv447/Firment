mod build;
mod edit_file;
mod flash;
mod glob;
mod grep;
mod list_dir;
mod read_file;
mod run;
mod shell;
mod symbols;
mod util;
mod verify;
mod write_file;

use firment_core::Tool;
use std::sync::Arc;

pub fn all() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(build::Build),
        Arc::new(read_file::ReadFile),
        Arc::new(run::Run),
        Arc::new(write_file::WriteFile),
        Arc::new(flash::Flash),
        Arc::new(edit_file::EditFile),
        Arc::new(list_dir::ListDir),
        Arc::new(glob::Glob),
        Arc::new(grep::Grep),
        Arc::new(shell::Shell),
        Arc::new(symbols::Symbols::new()),
        Arc::new(verify::Verify),
    ]
}
