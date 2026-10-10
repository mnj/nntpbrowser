//! egui shell for the NNTP browser.

mod cli;
mod format;
mod launch;
mod shell;
mod theme;

pub use cli::{parse_cli, run_cli, CliCommand};
pub use launch::launch;
pub use shell::Browser;
