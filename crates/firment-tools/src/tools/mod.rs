mod ask_user;
mod build;
mod debug;
mod decision;
mod edit_file;
pub(crate) mod elf_analyze;
pub(crate) mod flash;
mod glob;
mod grep;
mod hil;
mod html;
mod list_dir;
pub mod monitor;
mod periph_init;
mod pinmap;
mod read_file;
pub(crate) mod run;
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
        Arc::new(hil::Hil),
        Arc::new(periph_init::PeriphInit),
        Arc::new(pinmap::Pinmap),
        Arc::new(decision::Decision),
        Arc::new(write_file::WriteFile),
        Arc::new(flash::Flash),
        Arc::new(debug::Debug),
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
