mod edit_file;
mod glob;
mod grep;
mod list_dir;
mod read_file;
mod shell;
mod symbols;
mod util;
mod verify;
mod write_file;

use firment_core::Tool;
use std::sync::Arc;

pub fn all() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(read_file::ReadFile),
        Arc::new(write_file::WriteFile),
        Arc::new(edit_file::EditFile),
        Arc::new(list_dir::ListDir),
        Arc::new(glob::Glob),
        Arc::new(grep::Grep),
        Arc::new(shell::Shell),
        Arc::new(symbols::Symbols),
        Arc::new(verify::Verify),
    ]
}
