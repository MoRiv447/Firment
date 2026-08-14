mod ask_user;
mod build;
mod edit_file;
mod elf_analyze;
mod flash;
mod glob;
mod grep;
mod html;
mod list_dir;
pub mod monitor;
mod periph_init;
mod read_file;
mod run;
mod shell;
mod symbols;
mod task;
mod todo;
mod util;
mod verify;
mod web_fetch;
mod web_search;
mod write_file;

use firment_core::Tool;
use std::sync::Arc;

pub fn all() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ask_user::AskUser),
        Arc::new(build::Build),
        Arc::new(read_file::ReadFile),
        Arc::new(run::Run),
        Arc::new(monitor::Monitor),
        Arc::new(periph_init::PeriphInit),
        Arc::new(write_file::WriteFile),
        Arc::new(flash::Flash),
        Arc::new(edit_file::EditFile),
        Arc::new(elf_analyze::ElfAnalyze),
        Arc::new(list_dir::ListDir),
        Arc::new(glob::Glob),
        Arc::new(grep::Grep),
        Arc::new(shell::Shell),
        Arc::new(symbols::Symbols::new()),
        Arc::new(task::Task),
        Arc::new(todo::Todo),
        Arc::new(verify::Verify),
        Arc::new(web_fetch::WebFetch),
        Arc::new(web_search::WebSearch),
    ]
}
